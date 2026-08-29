// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The ms-graph adapter's WRITE path, executed as the real file in QuickJS.
//!
//! These tests load
//! `builtin-packages/ms-graph-adapter/content/functions/adapters/ms-graph/index.js`
//! from disk and run it through this runtime, with `raisin.http.fetch` scripted
//! by [`MockFunctionApi::with_http_responses`]. That is deliberate: the sync
//! engine's own `virtual_mount_sync` suite mocks the adapter entirely, so it
//! would pass no matter what this file does. The adapter's URL shape, headers
//! and status mapping are only observable here.

use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

fn adapter_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../builtin-packages/ms-graph-adapter/content/functions/adapters/ms-graph")
}

pub(super) fn adapter_source() -> String {
    let path = adapter_dir().join("index.js");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read adapter at {}: {e}", path.display()))
}

/// The adapter's sibling MODULES, keyed exactly as the runtime keys them: the
/// path relative to the function node, which for a flat function directory is
/// the bare filename.
///
/// This mirrors `load_sibling_files`, which lists the function node's children
/// in storage. Without it every test here would execute an entrypoint whose
/// imports resolve to nothing — and the resolver rejects an unknown specifier
/// rather than silently returning undefined, so the failure would be total
/// rather than subtle. `.mjs` is excluded deliberately: `index.test.mjs` is a
/// standalone node test, not a module this adapter imports.
pub(super) fn adapter_files() -> HashMap<String, String> {
    let dir = adapter_dir();
    let mut files = HashMap::new();
    for entry in std::fs::read_dir(&dir).expect("read adapter dir") {
        let path = entry.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name == "index.js" || !name.ends_with(".js") {
            continue;
        }
        files.insert(
            name.to_string(),
            std::fs::read_to_string(&path).expect("read adapter module"),
        );
    }
    assert!(
        !files.is_empty(),
        "no sibling modules found in {}",
        dir.display()
    );
    files
}

struct Run {
    output: Option<Value>,
    error: Option<String>,
    calls: Vec<Value>,
}

/// Invoke the real adapter with one operation, optionally scripting the HTTP
/// responses it will see.
async fn call_adapter(input: Value, responses: Vec<Value>) -> Run {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})).with_http_responses(responses));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(input);
    let metadata = FunctionMetadata::javascript("ms_graph_adapter");
    let result = runtime
        .execute(
            &adapter_source(),
            "handler",
            context,
            &metadata,
            api.clone() as Arc<dyn crate::api::FunctionApi>,
            adapter_files(),
        )
        .await
        .expect("runtime execute");
    Run {
        output: result.output,
        error: result.error.map(|e| e.message),
        calls: api.http_calls(),
    }
}

fn mount(resource: &str, principal: Option<&str>) -> Value {
    let mut config = json!({ "resource": resource });
    if let Some(p) = principal {
        config["principal"] = json!(p);
    }
    json!({
        "mount_id": "m1",
        "remote_root": "inbox",
        "config": config,
        "sync_config": config,
    })
}

fn credential() -> Value {
    json!({ "access_token": "TOKEN123", "provider_type": "ms-graph" })
}

/// The exact `update` params the engine's `push.rs` sends.
fn update_input(mount_value: Value, item_id: &str, etag: Option<&str>) -> Value {
    json!({
        "operation": "update",
        "credential": credential(),
        "mount": mount_value,
        "params": {
            "item_id": item_id,
            "payload": { "isRead": true },
            "fields": ["unread"],
            "etag": etag,
        },
    })
}

// ---------------------------------------------------------------------------

/// Write capability is declared for MAIL ONLY. Before this stage `can_write`
/// was a hardcoded `false` and `can_update`/`mutable_fields` were not emitted at
/// all, so every Graph mount resolved to
/// `WriteMode::Refused("adapter does not declare can_write, can_update")` — the
/// whole write path was unreachable against this provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capabilities_declare_update_for_mail_and_submit_for_mail_and_calendar() {
    let mail = call_adapter(
        json!({ "operation": "capabilities", "mount": mount("mail", None) }),
        vec![],
    )
    .await
    .output
    .expect("mail capabilities");
    assert_eq!(mail["can_write"], json!(true));
    assert_eq!(mail["can_update"], json!(true));
    // BOTH spellings of the read flag. `unread` is the global nodetype's
    // property and `is_read` is Graph-truth; a mount's write_config may be
    // authored in either, and the engine INTERSECTS the two lists — so
    // declaring only one silently drops the other's edits (an empty
    // intersection is refused loudly, a partial one is not).
    // …plus `importance`, which Graph accepts on the same message PATCH from a
    // closed set it also reports. The follow-up FLAG is imported but absent
    // here on purpose: writing it means writing a flag OBJECT with a status and
    // dates, and declaring a field the mapper cannot translate is how a push
    // resolves as supported and then throws at drain time.
    assert_eq!(
        mail["mutable_fields"],
        json!(["unread", "is_read", "importance"])
    );
    // NODE property names, never the Graph name: the mount's
    // write_config.mutable_fields is authored in node terms, and the survivors
    // of the intersection are what the mapper receives.
    assert_ne!(mail["mutable_fields"], json!(["isRead"]));

    // Mail is the only resource with an UPDATE, and both mail and calendar have
    // a SUBMIT (send / RSVP). They are separate declarations because they are
    // separate abilities: an outbox never patches an object and a state_only
    // inbox never issues a command, so folding them into one flag would make
    // each mount claim the other's capability.
    assert_eq!(mail["can_submit"], json!(true));
    assert_eq!(
        mail["supports_idempotency_key"],
        json!(false),
        "Graph has no idempotency key for sendMail; claiming one would change \
         nothing except what an operator believes about a duplicate"
    );

    let calendar = call_adapter(
        json!({ "operation": "capabilities", "mount": mount("calendar", None) }),
        vec![],
    )
    .await
    .output
    .expect("calendar capabilities");
    // Calendar declares the FULL mirror surface — create, update and delete are
    // all implemented — plus RSVP as a `submit` command. WHAT is writable is
    // pinned in `tests_ms_graph_calendar_write`; what this pins is that the
    // flags travel together, because a mirror mount is refused unless every op
    // it needs is declared.
    assert_eq!(calendar["can_write"], json!(true));
    assert_eq!(calendar["can_submit"], json!(true));
    assert_eq!(calendar["can_create"], json!(true));
    assert_eq!(calendar["can_update"], json!(true));
    assert_eq!(calendar["can_delete"], json!(true));
    assert!(calendar["mutable_fields"]
        .as_array()
        .is_some_and(|f| !f.is_empty()));

    let files = call_adapter(
        json!({ "operation": "capabilities", "mount": mount("files", None) }),
        vec![],
    )
    .await
    .output
    .expect("files capabilities");
    // Files became writable when the engine grew a byte channel (`content` on
    // create/update, plus the deferred-upload handshake). `accepts_content` is
    // the load-bearing flag: without it the engine offers no bytes and a mirror
    // would create empty objects at the provider.
    assert_eq!(files["can_write"], json!(true));
    assert_eq!(files["can_create"], json!(true));
    assert_eq!(files["can_update"], json!(true));
    assert_eq!(files["can_delete"], json!(true));
    assert_eq!(files["accepts_content"], json!(true));
    // A drive item is not a command surface — sending one is not a thing Graph
    // offers here, and declaring it would let a `submit` mount resolve.
    assert!(files.get("can_submit").is_none());
    assert_eq!(files["can_read"], json!(true));
    // Graph's DELETE is a recycle-bin move and there is no per-item permanent
    // delete, so `trash` is the only policy this adapter can honestly serve.
    assert_eq!(files["supports_trash"], json!(true));
    assert_eq!(files["default_delete_policy"], json!("trash"));
}

/// The PATCH is MAILBOX-scoped (`/users/{upn}/messages/{id}`), not folder-scoped
/// and not `/me` — that is what makes a shared mailbox writable. The body is the
/// mapper's payload VERBATIM, and `If-Match` carries the engine's etag.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_patches_the_mailbox_scoped_message_and_echoes_the_new_etag() {
    let run = call_adapter(
        update_input(
            mount("mail", Some("sales@contoso.com")),
            "AAMkAGI2=",
            Some("W/\"CQAAABYAAAA\""),
        ),
        vec![json!({
            "status": 200,
            "headers": { "etag": "W/\"IGNORED\"" },
            "body": { "id": "AAMkAGI2=", "@odata.etag": "W/\"CQAAABYAAAB\"", "isRead": true },
        })],
    )
    .await;

    assert_eq!(run.error, None);
    assert_eq!(run.calls.len(), 1, "exactly one Graph call");
    let call = &run.calls[0];
    assert_eq!(call["method"], json!("PATCH"));
    assert_eq!(
        call["url"],
        json!("https://graph.microsoft.com/v1.0/users/sales%40contoso.com/messages/AAMkAGI2%3D")
    );
    let headers = &call["options"]["headers"];
    assert_eq!(headers["Authorization"], json!("Bearer TOKEN123"));
    assert_eq!(headers["Content-Type"], json!("application/json"));
    assert_eq!(headers["If-Match"], json!("W/\"CQAAABYAAAA\""));
    // The mapper owns the node -> Graph translation; the adapter forwards it.
    assert_eq!(call["options"]["body"], json!({ "isRead": true }));

    let out = run.output.expect("update output");
    assert_eq!(out["external_id"], json!("AAMkAGI2="));
    // Read from the RESPONSE BODY's @odata.etag, which is what breaks the echo.
    assert_eq!(out["etag"], json!("W/\"CQAAABYAAAB\""));
}

/// A precondition failure is a CONFLICT, not a transient blip. `raiseForStatus`
/// maps no 412 at all, so before this stage a 412 fell through to a plain Error
/// and the engine retried the same stale If-Match forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_412_surfaces_as_conflict() {
    let run = call_adapter(
        update_input(mount("mail", None), "AAMk1", Some("W/\"OLD\"")),
        vec![json!({
            "status": 412,
            "headers": {},
            "body": { "error": { "code": "ErrorIrresolvableConflict", "message": "precondition failed" } },
        })],
    )
    .await;

    let err = run.error.expect("412 must throw");
    assert!(err.contains("[code=conflict]"), "got: {err}");
    // AdapterError::classify scans auth_expired, rate_limited, cursor_invalid,
    // config_error and THEN conflict, so any earlier token in the message would
    // misclassify this.
    for earlier in [
        "auth_expired",
        "rate_limited",
        "cursor_invalid",
        "config_error",
    ] {
        assert!(
            !err.contains(earlier),
            "conflict message contains {earlier}: {err}"
        );
    }
}

/// Graph message ids are not stable — a message that moves folders gets a new
/// id. `raiseForStatus` maps 404 to `config_error`, which is TERMINAL, so one
/// moved message would permanently mark a healthy mount misconfigured. `update`
/// owns its own 404 and reports "gone".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_404_is_gone_not_a_terminal_config_error() {
    let run = call_adapter(
        update_input(mount("mail", None), "AAMkStale", None),
        vec![json!({
            "status": 404,
            "headers": {},
            "body": { "error": { "code": "ErrorItemNotFound", "message": "not found" } },
        })],
    )
    .await;

    assert_eq!(run.error, None, "404 must not throw");
    assert!(
        matches!(run.output, None | Some(Value::Null)),
        "404 must report gone, got {:?}",
        run.output
    );
}

/// A 403 on a write is a missing WRITE SCOPE, not an expired token. Inheriting
/// `raiseForStatus`'s 401/403 -> auth_expired sends the operator to reconnect
/// the account, and reconnecting with the same consent cannot fix it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_403_diagnoses_the_missing_write_scope() {
    let run = call_adapter(
        update_input(mount("mail", None), "AAMk1", None),
        vec![json!({
            "status": 403,
            "headers": {},
            "body": { "error": { "code": "ErrorAccessDenied", "message": "Access is denied" } },
        })],
    )
    .await;

    let err = run.error.expect("403 must throw");
    assert!(err.contains("[code=config_error]"), "got: {err}");
    assert!(
        !err.contains("auth_expired"),
        "403 misdiagnosed as auth: {err}"
    );
    assert!(err.contains("Mail.ReadWrite"), "no scope guidance: {err}");
}

/// A stored `__etag` is not always an etag: `toExternalItem` falls back to
/// `lastModifiedDateTime`. Sending that as If-Match is a 400, and 400 is
/// terminal — it would mark a healthy mount misconfigured. So the header is
/// omitted unless the value has an etag's shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_omits_if_match_for_a_timestamp_shaped_etag() {
    let run = call_adapter(
        update_input(mount("mail", None), "AAMk1", Some("2026-01-01T00:00:00Z")),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "AAMk1" } })],
    )
    .await;

    assert_eq!(run.error, None);
    let headers = &run.calls[0]["options"]["headers"];
    assert!(
        headers.get("If-Match").is_none(),
        "a timestamp was sent as If-Match: {headers}"
    );
    // /me, because this mount names no principal.
    assert_eq!(
        run.calls[0]["url"],
        json!("https://graph.microsoft.com/v1.0/me/messages/AAMk1")
    );
    // No etag anywhere in the response: the engine keeps the stored one, and
    // __pushed_state (not the etag) is what suppresses the echo.
    assert_eq!(run.output.expect("output")["etag"], Value::Null);
}

/// An update with nothing to say is refused, terminally, and BEFORE the network.
///
/// Every resource is writable now — files last, once the engine could carry
/// bytes — so what this pins is the other half of the contract: a push the
/// adapter cannot turn into a provider call must throw `config_error` rather
/// than issue an empty request that the provider answers 400 to on every drain,
/// forever, for a change that can never converge.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_refuses_unwritable_resources_and_empty_payloads() {
    // A drive update with neither a metadata payload nor `content` has no
    // request to make: the bytes are what a file update usually IS, and their
    // absence here is the caller's mistake, not a provider limitation.
    let mut empty_drive = update_input(mount("files", None), "01ABC", None);
    empty_drive["params"]["payload"] = json!({});
    let run = call_adapter(empty_drive, vec![]).await;
    let err = run.error.expect("an empty drive update must throw");
    assert!(err.contains("[code=config_error]"), "files: {err}");
    assert!(run.calls.is_empty(), "files reached the network");

    let mut input = update_input(mount("mail", None), "AAMk1", None);
    input["params"]["payload"] = json!({});
    let run = call_adapter(input, vec![]).await;
    let err = run.error.expect("an empty PATCH must be refused");
    assert!(err.contains("[code=config_error]"), "got: {err}");
    assert!(run.calls.is_empty(), "an empty PATCH reached the network");
}

// ---------------------------------------------------------------------------
// submit: the outbox (stage 10)
// ---------------------------------------------------------------------------

/// The exact `submit` params the engine's `submit_step.rs` sends.
fn submit_input(mount_value: Value, action: &str, body: Value, external_id: Option<&str>) -> Value {
    json!({
        "operation": "submit",
        "credential": credential(),
        "mount": mount_value,
        "params": {
            "payload": { "action": action, "body": body },
            "external_id": external_id,
            "idempotency_key": "m1:node1:attempt1",
        },
    })
}

/// A send is ONE mailbox-scoped POST, with the mapper's body verbatim.
///
/// Mailbox-scoped for the same reason the PATCH is: a literal `/me` here would
/// send FROM the connected account while the mount is configured for a shared
/// mailbox — silently, and to the recipient it is the wrong sender.
///
/// One call, not three. Graph also offers createReply/createForward + send, and
/// each extra round trip is another window in which the engine cannot tell
/// whether the message left. The whole at-most-once protocol exists to bound
/// that window to one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_sends_one_mailbox_scoped_post_with_the_mappers_body() {
    let body = json!({
        "message": {
            "subject": "hi",
            "body": { "contentType": "HTML", "content": "<p>hi</p>" },
            "toRecipients": [{ "emailAddress": { "address": "a@b.com" } }],
        },
        "saveToSentItems": true,
    });
    let run = call_adapter(
        submit_input(
            mount("mail", Some("sales@contoso.com")),
            "send",
            body.clone(),
            None,
        ),
        // 202 Accepted with an EMPTY body — what Graph actually answers.
        vec![json!({ "status": 202, "headers": {}, "body": null })],
    )
    .await;

    assert_eq!(run.error, None);
    assert_eq!(run.calls.len(), 1, "exactly one Graph call per command");
    let call = &run.calls[0];
    assert_eq!(call["method"], json!("POST"));
    assert_eq!(
        call["url"],
        json!("https://graph.microsoft.com/v1.0/users/sales%40contoso.com/sendMail")
    );
    assert_eq!(
        call["options"]["body"], body,
        "the body is forwarded verbatim"
    );

    // An OBJECT must come back even though Graph returned nothing: a null is
    // read by the engine as "the outcome is unknown", which would park a send
    // that plainly succeeded.
    let out = run.output.expect("submit output");
    assert!(out.is_object(), "got {out:?}");
    assert_eq!(out["external_id"], Value::Null);
}

/// Reply, reply-all and forward route to the provider's own actions against the
/// message being answered — which is why they need no engine support at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_routes_reply_and_forward_through_the_target_message() {
    for (action, verb) in [
        ("reply", "reply"),
        ("reply_all", "replyAll"),
        ("forward", "forward"),
    ] {
        let run = call_adapter(
            submit_input(
                mount("mail", None),
                action,
                json!({ "message": { "body": { "contentType": "Text", "content": "ok" } } }),
                Some("AAMkTARGET="),
            ),
            vec![json!({ "status": 202, "headers": {}, "body": null })],
        )
        .await;
        assert_eq!(run.error, None, "{action}");
        assert_eq!(
            run.calls[0]["url"],
            json!(format!(
                "https://graph.microsoft.com/v1.0/me/messages/AAMkTARGET%3D/{verb}"
            )),
            "{action}"
        );
    }
}

/// An RSVP is the same command protocol pointed at an event.
///
/// `tentative` maps to Graph's `tentativelyAccept`; the node's vocabulary is the
/// user's, not the provider's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_responds_to_an_event_for_a_calendar_mount() {
    for (action, verb) in [
        ("accept", "accept"),
        ("decline", "decline"),
        ("tentative", "tentativelyAccept"),
    ] {
        let run = call_adapter(
            submit_input(
                mount("calendar", None),
                action,
                json!({ "comment": "see you", "sendResponse": true }),
                Some("EVT1"),
            ),
            vec![json!({ "status": 202, "headers": {}, "body": null })],
        )
        .await;
        assert_eq!(run.error, None, "{action}");
        assert_eq!(
            run.calls[0]["url"],
            json!(format!(
                "https://graph.microsoft.com/v1.0/me/events/EVT1/{verb}"
            )),
            "{action}"
        );
        assert_eq!(
            run.calls[0]["options"]["body"],
            json!({ "comment": "see you", "sendResponse": true })
        );
    }
}

/// A 404 on a submit is TERMINAL, and deliberately not the `null` that the
/// update path returns for the same status.
///
/// On the update path a 404 means "this message moved and got a new id" and the
/// engine settles the node. There is no such recovery for a command: the message
/// being replied to is gone, so the reply can never be issued as written.
/// Returning null would park it at `unknown` — i.e. tell the operator we might
/// have sent something, which is strictly false.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_404_fails_definitively_rather_than_reporting_gone() {
    let run = call_adapter(
        submit_input(
            mount("mail", None),
            "reply",
            json!({ "message": { "subject": "re" } }),
            Some("AAMkGONE"),
        ),
        vec![json!({
            "status": 404,
            "headers": {},
            "body": { "error": { "code": "ErrorItemNotFound", "message": "not found" } },
        })],
    )
    .await;

    let err = run.error.expect("a 404 on a command must throw");
    assert!(err.contains("[code=config_error]"), "got: {err}");
    assert!(
        run.output.is_none() || run.output == Some(Value::Null),
        "a definitive rejection must not also produce a result"
    );
}

/// A 403 on a send is a missing SEND scope, named. It is the first thing a new
/// outbox hits: the connector requests read scopes, so composing works and every
/// send 403s — and reconnecting with the same consent cannot fix it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_403_diagnoses_the_missing_send_scope() {
    let run = call_adapter(
        submit_input(
            mount("mail", None),
            "send",
            json!({ "message": { "toRecipients": [] } }),
            None,
        ),
        vec![json!({
            "status": 403,
            "headers": {},
            "body": { "error": { "code": "ErrorAccessDenied", "message": "Access is denied" } },
        })],
    )
    .await;

    let err = run.error.expect("403 must throw");
    assert!(err.contains("[code=config_error]"), "got: {err}");
    assert!(!err.contains("auth_expired"), "misdiagnosed as auth: {err}");
    assert!(err.contains("Mail.Send"), "no scope guidance: {err}");
}

/// Everything refusable is refused BEFORE the network, terminally.
///
/// A command that cannot be issued as written must never reach the provider on
/// the chance it works: the engine has already claimed it, so a call that gets
/// through is a call whose outcome has to be reasoned about.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_refuses_unroutable_commands_without_calling_the_provider() {
    let cases = vec![
        // An action this adapter does not know.
        submit_input(
            mount("mail", None),
            "publish",
            json!({ "message": {} }),
            None,
        ),
        // A reply with no target.
        submit_input(mount("mail", None), "reply", json!({ "message": {} }), None),
        // An RSVP with no event.
        submit_input(
            mount("calendar", None),
            "accept",
            json!({ "comment": "x" }),
            None,
        ),
        // A resource that cannot issue commands at all.
        submit_input(mount("files", None), "send", json!({ "message": {} }), None),
        // An empty body: an empty send is not a send.
        submit_input(mount("mail", None), "send", json!({}), None),
    ];
    for input in cases {
        let label = input["params"]["payload"]["action"].clone();
        let run = call_adapter(input, vec![]).await;
        let err = run
            .error
            .unwrap_or_else(|| panic!("{label} must be refused"));
        assert!(err.contains("[code=config_error]"), "{label}: {err}");
        assert!(
            run.calls.is_empty(),
            "{label} reached the provider: {:?}",
            run.calls
        );
    }
}

// ---------------------------------------------------------------------------
// subscribe: changeType is per RESOURCE
// ---------------------------------------------------------------------------

/// Graph rejects the whole subscription when `changeType` names a value the
/// resource does not support, so one shared value cannot serve all three
/// surfaces.
///
/// `driveItem` — what a FILES mount subscribes to (`{drive}/root`) — supports
/// **`updated` only**: Microsoft models a new file as an update of its parent,
/// so there is no `created` to ask for. Asking anyway produced
/// `Invalid 'changeType' attribute: 'created'` and every OneDrive/SharePoint
/// mount on a webhook or hybrid sync silently fell back to polling, reporting a
/// config_error on each attempt.
///
/// Asserted per resource because the failure is invisible until a provider
/// rejects it — there is nothing in the request itself that looks wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_asks_for_the_change_types_each_resource_actually_supports() {
    for (resource, expected) in [
        ("mail", "created,updated"),
        ("calendar", "created,updated"),
        ("files", "updated"),
    ] {
        let run = call_adapter(
            json!({
                "operation": "subscribe",
                "credential": { "access_token": "TOKEN123", "provider_type": "ms-graph" },
                "mount": mount(resource, None),
                "params": { "notification_url": "https://example.test/notify" },
            }),
            vec![json!({
                "status": 201,
                "headers": {},
                "body": { "id": "SUB1", "expirationDateTime": "2026-08-08T00:00:00Z" }
            })],
        )
        .await;
        assert!(run.error.is_none(), "{resource}: {:?}", run.error);
        assert_eq!(
            run.calls[0]["options"]["body"]["changeType"],
            json!(expected),
            "{resource} asked for the wrong change types"
        );
    }
}
