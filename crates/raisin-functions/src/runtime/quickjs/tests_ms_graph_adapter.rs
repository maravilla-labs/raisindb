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

fn adapter_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../builtin-packages/ms-graph-adapter/content/functions/adapters/ms-graph/index.js",
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read adapter at {}: {e}", path.display()))
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
            HashMap::new(),
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
async fn capabilities_declare_write_for_mail_only() {
    let mail = call_adapter(
        json!({ "operation": "capabilities", "mount": mount("mail", None) }),
        vec![],
    )
    .await
    .output
    .expect("mail capabilities");
    assert_eq!(mail["can_write"], json!(true));
    assert_eq!(mail["can_update"], json!(true));
    assert_eq!(mail["mutable_fields"], json!(["unread"]));
    // The NODE property name, never the Graph name: the engine intersects this
    // with the mount's write_config.mutable_fields, which is authored in node
    // terms, and hands the survivors to the mapper.
    assert_ne!(mail["mutable_fields"], json!(["isRead"]));

    for resource in ["calendar", "files"] {
        let caps = call_adapter(
            json!({ "operation": "capabilities", "mount": mount(resource, None) }),
            vec![],
        )
        .await
        .output
        .unwrap_or_else(|| panic!("{resource} capabilities"));
        assert_eq!(
            caps["can_write"],
            json!(false),
            "{resource} must stay read-only"
        );
        assert!(
            caps.get("can_update").is_none(),
            "{resource} declared can_update"
        );
        assert!(
            caps.get("mutable_fields").is_none(),
            "{resource} declared mutable_fields"
        );
        assert_eq!(caps["can_read"], json!(true));
    }
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

/// Calendar and files have no `update`. Refused up front and terminally, so the
/// engine stops rather than retrying an unsupported request forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_refuses_non_mail_resources_and_empty_payloads() {
    for resource in ["calendar", "files"] {
        let run = call_adapter(update_input(mount(resource, None), "x", None), vec![]).await;
        let err = run
            .error
            .unwrap_or_else(|| panic!("{resource} update must throw"));
        assert!(err.contains("[code=config_error]"), "{resource}: {err}");
        assert!(run.calls.is_empty(), "{resource} reached the network");
    }

    let mut input = update_input(mount("mail", None), "AAMk1", None);
    input["params"]["payload"] = json!({});
    let run = call_adapter(input, vec![]).await;
    let err = run.error.expect("an empty PATCH must be refused");
    assert!(err.contains("[code=config_error]"), "got: {err}");
    assert!(run.calls.is_empty(), "an empty PATCH reached the network");
}
