// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The google-calendar WRITE path — mapper and adapter, both as the real files
//! in QuickJS.
//!
//! The engine's own suite mocks both halves, so it would pass no matter what
//! these files emit. What is only observable here is what actually reaches
//! Google: whether an invitation email goes out, which calendar a create lands
//! in, and which nodes the mapper REFUSES to translate outward.
//!
//! Before this the adapter had NO write operation at all — `can_write: false`
//! and a dispatch table with no create/update/delete case — so a calendar mirror
//! mount resolved to `Refused` against both shipped calendar providers.

use super::tests_google_calendar_adapter::{call_adapter, mount};
// `function_files` is provider-neutral despite its home; it reads a function
// node's sibling modules off disk the way the engine does from storage.
use super::tests_ms_graph_adapter::function_files;
use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

fn mapper_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../builtin-packages/google-calendar-adapter/content/functions/mappers/google-calendar-default",
    )
}

fn mapper_source() -> String {
    let path = mapper_dir().join("index.js");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read mapper at {}: {e}", path.display()))
}

/// The mapper's sibling MODULES, for the same reason the ms-graph calendar
/// harness has them: passing `HashMap::new()` supplies LESS than production, so
/// an `import` the engine resolves fine fails here and nowhere else — which is
/// what silently forbade splitting the ms-graph calendar mapper for as long as
/// it did. Empty today, because this mapper is still one file; the point is that
/// it stops being a trap the day it is not.
fn mapper_files() -> HashMap<String, String> {
    function_files(&mapper_dir())
}

/// A mapper is pure and I/O-free, so it gets no scripted HTTP at all — and if it
/// ever reached for the network, `http_calls` would catch it.
async fn call_mapper(input: Value) -> (Option<Value>, Vec<Value>) {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(input);
    let metadata = FunctionMetadata::javascript("google_calendar_mapper");
    let result = runtime
        .execute(
            &FunctionCode::from(mapper_source()),
            "handler",
            context,
            &metadata,
            api.clone() as Arc<dyn crate::api::FunctionApi>,
            mapper_files(),
        )
        .await
        .expect("runtime execute");
    if let Some(err) = result.error {
        panic!("mapper threw: {}", err.message);
    }
    (result.output, api.http_calls())
}

async fn to_external(properties: Value, fields: Value, intent: &str) -> Option<Value> {
    let (out, http) = call_mapper(json!({
        "operation": "to_external",
        "node": { "properties": properties },
        "mount": mount(false),
        "fields": fields,
        "intent": intent,
    }))
    .await;
    assert!(http.is_empty(), "a mapper must perform no I/O");
    out.filter(|v| !v.is_null())
}

async fn payload_of(properties: Value, intent: &str) -> Value {
    to_external(properties, Value::Null, intent)
        .await
        .expect("mapper translated the node")["payload"]
        .clone()
}

fn timed_event() -> Value {
    json!({
        "title": "Standup",
        "start_utc": "2026-08-11T07:00:00Z",
        "end_utc": "2026-08-11T07:15:00Z",
        "start_local": "2026-08-11T09:00:00",
        "end_local": "2026-08-11T09:15:00",
        "timezone": "Europe/Zurich",
        "all_day": false,
        "recurrence_type": "single",
    })
}

// ---------------------------------------------------------------------------
// mapper

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_google_mapper_now_declares_the_write_direction() {
    let (caps, _) = call_mapper(json!({ "operation": "mapper_capabilities", "mount": {} })).await;
    assert_eq!(caps.expect("capabilities")["to_external"], json!(true));
}

/// The payoff of making RFC 5545 the canonical shape: Google's `recurrence` IS
/// an array of content lines, so the column travels VERBATIM in both directions.
/// No pattern vocabulary to translate, no UNTIL arithmetic — the ms-graph mapper
/// spends ~150 lines on the same column.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recurrence_travels_verbatim_in_both_directions() {
    let mut props = timed_event();
    props["recurrence_type"] = json!("series_master");
    props["recurrence"] = json!([
        "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261231T210000Z",
        "EXDATE;TZID=Europe/Zurich:20260915T090000",
    ]);
    let payload = payload_of(props, "update").await;
    assert_eq!(
        payload["recurrence"],
        json!([
            "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261231T210000Z",
            "EXDATE;TZID=Europe/Zurich:20260915T090000",
        ]),
        "the lines must survive untouched — including EXDATE, which has no \
         patternedRecurrence equivalent at all"
    );
}

/// Same rule as the Graph side, for the same provider reason: an exception has
/// its own event id once it exists (so a PATCH is ordinary), but Google mints one
/// only by patching an INSTANCE of a live series. POSTing /events for one creates
/// a standalone event the series still overlaps.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exception_updates_but_never_creates_and_an_occurrence_never_travels() {
    let mut props = timed_event();
    props["series_master_external_id"] = json!("M1");

    props["recurrence_type"] = json!("exception");
    assert!(to_external(props.clone(), Value::Null, "create")
        .await
        .is_none());
    assert!(to_external(props.clone(), Value::Null, "update")
        .await
        .is_some());

    props["recurrence_type"] = json!("occurrence");
    assert!(to_external(props.clone(), Value::Null, "create")
        .await
        .is_none());
    assert!(to_external(props, Value::Null, "update").await.is_none());
}

/// The zone question. Google reads a `dateTime` with no offset in the request's
/// `timeZone`, so a naive local paired with no zone would be interpreted in the
/// CALENDAR's zone and silently move the event. The UTC value is preferred
/// because it is unambiguous — and the zone still travels beside it, because
/// Google expands the recurrence rule server-side in that zone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_time_carries_its_zone_and_a_naive_local_alone_is_never_guessed() {
    let payload = payload_of(timed_event(), "create").await;
    assert_eq!(payload["start"]["dateTime"], json!("2026-08-11T07:00:00Z"));
    assert_eq!(payload["start"]["timeZone"], json!("Europe/Zurich"));

    let mut props = timed_event();
    props["start_utc"] = Value::Null;
    props["end_utc"] = Value::Null;
    props["timezone"] = Value::Null;
    assert!(
        to_external(props, Value::Null, "create").await.is_none(),
        "a create with no resolvable instant must decline, not guess"
    );
}

/// All-day flips the representation of BOTH ends at once (`date`, not
/// `dateTime`), which is one reason the time group never travels in pieces.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_all_day_event_uses_bare_dates() {
    let mut props = timed_event();
    props["all_day"] = json!(true);
    props["start_local"] = json!("2026-08-11");
    props["end_local"] = json!("2026-08-12");
    let payload = payload_of(props, "create").await;
    assert_eq!(payload["start"], json!({ "date": "2026-08-11" }));
    assert_eq!(payload["end"], json!({ "date": "2026-08-12" }));
    assert!(payload["start"].get("dateTime").is_none());
}

/// A time field drags the whole group; an unrelated field does not drag time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_allow_list_is_honoured_except_for_the_time_group() {
    let payload = to_external(timed_event(), json!(["start_utc"]), "update")
        .await
        .expect("start is writable")["payload"]
        .clone();
    assert!(payload.get("start").is_some());
    assert!(payload.get("end").is_some(), "{payload:#?}");
    assert!(payload.get("summary").is_none(), "{payload:#?}");

    let payload = to_external(timed_event(), json!(["title"]), "update")
        .await
        .expect("title is writable")["payload"]
        .clone();
    assert_eq!(payload["summary"], json!("Standup"));
    assert!(payload.get("start").is_none(), "{payload:#?}");
}

/// `out_of_office` and `working_elsewhere` are Graph concepts. Google's
/// transparency has exactly two values, and flattening the others onto `opaque`
/// would silently rewrite the user's choice on every push — so an unmappable
/// value emits nothing at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unmappable_show_as_emits_nothing_rather_than_the_nearest_value() {
    let mut props = timed_event();
    props["show_as"] = json!("free");
    assert_eq!(
        payload_of(props.clone(), "update").await["transparency"],
        json!("transparent")
    );

    props["show_as"] = json!("out_of_office");
    let payload = payload_of(props, "update").await;
    assert!(payload.get("transparency").is_none(), "{payload:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attendees_map_to_google_shape_and_an_empty_list_is_meaningful() {
    let mut props = timed_event();
    props["attendees"] = json!([
        { "email": "a@example.com", "name": "A", "type": "required" },
        { "email": "b@example.com", "name": null, "type": "optional" },
        { "email": null, "name": "no address", "type": "required" },
    ]);
    let payload = payload_of(props.clone(), "create").await;
    assert_eq!(
        payload["attendees"],
        json!([
            { "email": "a@example.com", "displayName": "A" },
            { "email": "b@example.com", "optional": true },
        ]),
        "an attendee with no address cannot be invited and is dropped"
    );

    props["attendees"] = json!([]);
    assert_eq!(payload_of(props, "create").await["attendees"], json!([]));
}

/// Nothing to say is `null`, not `{}`. An empty PATCH still bumps the event's
/// etag, invalidating every stored one and making the next delta re-deliver it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_with_no_writable_field_emits_null() {
    assert!(to_external(timed_event(), json!(["url"]), "update")
        .await
        .is_none());
}

// ---------------------------------------------------------------------------
// adapter

fn write_input(op: &str, params: Value, send_updates: Option<&str>) -> Value {
    let mut m = mount(false);
    if let Some(s) = send_updates {
        m["sync_config"]["send_updates"] = json!(s);
    }
    json!({
        "operation": op,
        "credential": { "access_token": "TOKEN123" },
        "mount": m,
        "params": params,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_google_calendar_mount_now_declares_the_full_mirror_surface() {
    let caps = call_adapter(
        json!({ "operation": "capabilities", "mount": mount(false) }),
        vec![],
    )
    .await
    .output
    .expect("capabilities");
    assert_eq!(caps["can_write"], json!(true));
    assert_eq!(caps["can_create"], json!(true));
    assert_eq!(caps["can_update"], json!(true));
    assert_eq!(caps["can_delete"], json!(true));

    // Google has NO trash for events: a delete is immediate and unrecoverable.
    // Declaring one would let a mount configure a soft delete this provider
    // cannot perform — and the engine would report success for it. The default
    // is `detach`, so a local delete does not propagate until an operator types
    // `purge`.
    assert_eq!(caps["supports_trash"], json!(false));
    assert_eq!(caps["default_delete_policy"], json!("detach"));

    // `can_submit` IS declared now, and the mechanism is worth stating because
    // it is not Graph's. Google exposes no accept/decline endpoint: an RSVP is
    // `events.patch` of the caller's own attendee row. It stays a COMMAND
    // rather than a property edit for the same reason it is one on Graph —
    // responding notifies the organizer, and an irreversible externally visible
    // effect must not hide behind a field change.
    //
    // The adapter's patch must read-modify-write the whole attendees array:
    // Google's array fields overwrite completely, so a patch carrying only the
    // caller's row would DELETE every other guest from the meeting.
    assert_eq!(caps["can_submit"], json!(true), "{caps:#?}");
}

/// NOBODY GETS EMAILED BY DEFAULT.
///
/// Google mails every attendee when an event with attendees is created, moved or
/// deleted. A sync engine mirroring a node is not a person deciding to notify
/// twelve people, and the notification is irreversible and externally visible —
/// the same property that makes an RSVP a `submit` command rather than a
/// property edit. A mount that wants invitations has to say so out loud.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_write_notifies_attendees_unless_the_mount_opts_in() {
    for op in ["create", "update", "delete"] {
        let run = call_adapter(
            write_input(
                op,
                json!({ "item_id": "E1", "payload": { "summary": "x" }, "policy": "purge" }),
                None,
            ),
            vec![json!({ "status": 200, "headers": {}, "body": { "id": "E1" } })],
        )
        .await;
        let url = run.calls[0]["url"].as_str().expect("a request");
        assert!(url.contains("sendUpdates=none"), "{op}: {url}");
    }

    let run = call_adapter(
        write_input(
            "create",
            json!({ "payload": { "summary": "x" } }),
            Some("all"),
        ),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "E1" } })],
    )
    .await;
    assert!(run.calls[0]["url"]
        .as_str()
        .unwrap()
        .contains("sendUpdates=all"));

    // An unrecognized value falls back to `none` rather than reaching Google as
    // a 400 — or, worse, being read by Google as a default that notifies.
    let run = call_adapter(
        write_input(
            "create",
            json!({ "payload": { "summary": "x" } }),
            Some("everyone"),
        ),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "E1" } })],
    )
    .await;
    assert!(run.calls[0]["url"]
        .as_str()
        .unwrap()
        .contains("sendUpdates=none"));
}

/// A create POSTs into the MOUNT's calendar and returns the id the engine adopts
/// the node with. Without an id the node would look synced, carry an id no event
/// has, and the next reconcile would create a SECOND copy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_posts_into_the_mounts_calendar_and_returns_the_new_id() {
    let run = call_adapter(
        write_input(
            "create",
            json!({ "payload": { "summary": "Standup" }, "parent_id": "team@group.calendar.google.com" }),
            None,
        ),
        vec![json!({
            "status": 200,
            "headers": {},
            "body": { "id": "EV1", "etag": "\"v1\"" }
        })],
    )
    .await;
    let call = &run.calls[0];
    assert_eq!(call["method"], json!("POST"));
    assert!(
        call["url"]
            .as_str()
            .unwrap()
            .contains("/calendars/team%40group.calendar.google.com/events"),
        "{}",
        call["url"]
    );
    let out = run.output.expect("create output");
    assert_eq!(out["external_id"], json!("EV1"));
    assert_eq!(out["etag"], json!("\"v1\""));
}

/// An accepted create with no id is a FAILURE, not a success: adopting the node
/// anyway would leave an id no event has and a duplicate at the next reconcile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_that_returns_no_id_fails_loudly() {
    let run = call_adapter(
        write_input("create", json!({ "payload": { "summary": "x" } }), None),
        vec![json!({ "status": 200, "headers": {}, "body": {} })],
    )
    .await;
    let err = run.error.expect("must throw");
    assert!(err.contains("no id"), "{err}");
}

/// PATCH, not PUT: an update carries an allow-listed subset, and PUT would clear
/// every field the mount does not push.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_patches_and_sends_if_match_only_for_a_real_etag() {
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "summary": "Renamed" }, "etag": "\"v1\"" }),
            None,
        ),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "EV1", "etag": "\"v2\"" } })],
    )
    .await;
    let call = &run.calls[0];
    assert_eq!(call["method"], json!("PATCH"));
    assert_eq!(call["options"]["headers"]["If-Match"], json!("\"v1\""));
    assert_eq!(run.output.expect("output")["etag"], json!("\"v2\""));

    // `toExternalItem` falls back to `updated` (an ISO timestamp) when Google
    // sent no etag, so a healthy node can carry a value that is not an etag at
    // all. Sending it as If-Match is a 400, and 400 is TERMINAL.
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "summary": "x" }, "etag": "2026-08-11T09:00:00Z" }),
            None,
        ),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "EV1" } })],
    )
    .await;
    assert!(
        run.calls[0]["options"]["headers"].get("If-Match").is_none(),
        "{:#?}",
        run.calls[0]["options"]["headers"]
    );
}

/// A 412 is the mount's CONFLICT policy to resolve, never a retry — the retry
/// sends the same stale If-Match and fails identically. The message must not
/// contain an earlier classifier token, because `AdapterError::classify` scans
/// for auth_expired / rate_limited / cursor_invalid / config_error BEFORE
/// conflict.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_precondition_failure_is_a_conflict_not_a_retry() {
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "summary": "x" }, "etag": "\"stale\"" }),
            None,
        ),
        vec![json!({ "status": 412, "headers": {}, "body": {} })],
    )
    .await;
    let err = run.error.expect("must throw");
    assert!(err.contains("[code=conflict]"), "{err}");
}

/// A write 403 is a missing SCOPE, not a stale token. Inheriting the read path's
/// 403 → auth_expired would send the operator to reconnect the account, and
/// reconnecting with the same read-only consent cannot fix it — the wrong
/// diagnosis costs more than the failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_403_names_the_missing_scope_instead_of_blaming_the_token() {
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "summary": "x" } }),
            None,
        ),
        vec![json!({
            "status": 403,
            "headers": {},
            "body": { "error": { "errors": [{ "reason": "insufficientPermissions" }] } }
        })],
    )
    .await;
    let err = run.error.expect("must throw");
    assert!(err.contains("[code=config_error]"), "{err}");
    assert!(
        err.contains("calendar.events"),
        "must name the scope: {err}"
    );

    // A rate-limit 403 keeps the READ mapping — it is not a scope problem and
    // must requeue rather than mark the mount misconfigured.
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "summary": "x" } }),
            None,
        ),
        vec![json!({
            "status": 403,
            "headers": {},
            "body": { "error": { "errors": [{ "reason": "userRateLimitExceeded" }] } }
        })],
    )
    .await;
    let err = run.error.expect("must throw");
    assert!(err.contains("[code=rate_limited]"), "{err}");
}

/// A vanished event settles an update and completes a delete. Google answers 410
/// for an event deleted earlier, which makes "already gone" the COMMON case on
/// the delete path rather than an edge one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gone_event_settles_an_update_and_completes_a_delete() {
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "summary": "x" } }),
            None,
        ),
        vec![json!({ "status": 404, "headers": {}, "body": {} })],
    )
    .await;
    assert_eq!(run.output, Some(Value::Null));

    let run = call_adapter(
        write_input(
            "delete",
            json!({ "item_id": "EV1", "policy": "purge" }),
            None,
        ),
        vec![json!({ "status": 410, "headers": {}, "body": {} })],
    )
    .await;
    assert_eq!(run.output.expect("output")["deleted"], json!(true));
}

/// `trash` is refused rather than served as a purge. The engine already refuses
/// it at policy resolution (`supports_trash: false`), so this is the second
/// gate — it catches an engine resolving against a stale cached capability
/// record. Promoting a trash request to an irreversible delete is the single
/// worst substitution available here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trash_is_refused_because_google_has_no_bin() {
    let run = call_adapter(
        write_input(
            "delete",
            json!({ "item_id": "EV1", "policy": "trash" }),
            None,
        ),
        vec![],
    )
    .await;
    let err = run.error.expect("must throw");
    assert!(err.contains("[code=config_error]"), "{err}");
    assert!(err.contains("delete_policy"), "must name the fix: {err}");
    assert!(run.calls.is_empty(), "nothing may reach Google");
}
