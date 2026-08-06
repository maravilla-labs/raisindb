// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The ms-graph CALENDAR write path — mapper and adapter, both as the real files
//! in QuickJS.
//!
//! The engine's own suite mocks both halves, so it would pass no matter what
//! these files emit. What is only observable here is what actually reaches
//! Microsoft 365: which URL a create posts to, whether a recurrence rule
//! survives the round trip through Graph's much narrower pattern vocabulary,
//! and — most of all — which nodes the mapper REFUSES to translate outward. A
//! mapper that guesses on this path silently reschedules real meetings.

use super::tests_ms_graph_adapter::{adapter_files, adapter_source};
use super::tests_ms_graph_calendar::{call_adapter, calendar_mount};
use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

fn mapper_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../builtin-packages/ms-graph-adapter/content/functions/mappers/ms-graph-calendar/index.js",
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read mapper at {}: {e}", path.display()))
}

/// A mapper is pure and I/O-free, so it gets no scripted HTTP at all — and if it
/// ever reached for the network, `http_calls` would catch it.
async fn call_mapper(input: Value) -> (Option<Value>, Vec<Value>) {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(input);
    let metadata = FunctionMetadata::javascript("ms_graph_calendar");
    let result = runtime
        .execute(
            &mapper_source(),
            "handler",
            context,
            &metadata,
            api.clone() as Arc<dyn crate::api::FunctionApi>,
            HashMap::new(),
        )
        .await
        .expect("runtime execute");
    if let Some(err) = result.error {
        panic!("mapper threw: {}", err.message);
    }
    (result.output, api.http_calls())
}

/// The `to_external` input the engine's write drain sends. `fields: null` is a
/// create or a mirror update with no allow-list; `intent` is what separates the
/// two, and the mapper is the only thing that can act on the difference.
async fn to_external(properties: Value, fields: Value, intent: &str) -> Option<Value> {
    let (out, http) = call_mapper(json!({
        "operation": "to_external",
        "node": { "properties": properties },
        "mount": calendar_mount(false),
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

/// A plain, fully-specified single event — the baseline every refusal below is
/// measured against.
fn timed_event() -> Value {
    json!({
        "title": "Standup",
        "start_local": "2026-08-11T09:00:00",
        "end_local": "2026-08-11T09:15:00",
        "timezone": "Europe/Berlin",
        "all_day": false,
        "recurrence_type": "single",
    })
}

// ---------------------------------------------------------------------------
// mapper: the probe

/// Without this the mount is reported read-only and a calendar mirror never
/// resolves at all — the mapper's own capability is ANDed with the adapter's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_calendar_mapper_now_declares_the_write_direction() {
    let (caps, _) = call_mapper(json!({ "operation": "mapper_capabilities", "mount": {} })).await;
    assert_eq!(caps.expect("capabilities")["to_external"], json!(true));
}

// ---------------------------------------------------------------------------
// mapper: what it refuses

/// A derived occurrence is a PROJECTION of its master, regenerated from the
/// rule. Creating one at the provider mints a duplicate meeting that the next
/// rebuild disowns; updating one writes an edit the next rebuild discards. Both
/// directions, both intents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_occurrence_is_never_translated_outward() {
    let mut props = timed_event();
    props["recurrence_type"] = json!("occurrence");
    props["series_master_external_id"] = json!("S1");
    assert!(to_external(props.clone(), Value::Null, "create")
        .await
        .is_none());
    assert!(to_external(props, Value::Null, "update").await.is_none());
}

/// An exception is UPDATABLE and NOT CREATABLE, and `intent` is the only thing
/// that can tell the two apart — `fields` is empty for a mirror update as well
/// as for a create.
///
/// The asymmetry is a fact about Graph, not a policy: an exception has its own
/// event id once it exists, so PATCHing it is ordinary, but Graph mints one only
/// by diverging an occurrence of a live series. POSTing /events for one creates a
/// standalone event the series still overlaps — a duplicate in the user's
/// calendar, which is exactly what the engine's create path cannot detect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exception_updates_but_never_creates() {
    let mut props = timed_event();
    props["recurrence_type"] = json!("exception");
    props["series_master_external_id"] = json!("S1");
    assert!(
        to_external(props.clone(), Value::Null, "create")
            .await
            .is_none(),
        "an exception cannot be POSTed into existence"
    );
    assert!(
        to_external(props, Value::Null, "update").await.is_some(),
        "an exception that already exists is an ordinary PATCH"
    );
}

/// The zone question, which is the one that corrupts real data silently.
///
/// `start_local` is a NAIVE wall clock and `timezone` is legitimately null when
/// `to_node` could not map a Windows zone name. Pairing the two as UTC would
/// move every event in a non-UTC calendar by its offset — plausible-looking and
/// wrong. With no UTC value to fall back on there is nothing honest to send, so
/// a create declines outright rather than storing an event at the wrong time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_naive_local_time_with_no_zone_is_never_guessed_as_utc() {
    let mut props = timed_event();
    props["timezone"] = Value::Null;
    assert!(
        to_external(props.clone(), Value::Null, "create")
            .await
            .is_none(),
        "a create with no resolvable instant must decline, not guess"
    );

    // The same node WITH a UTC value resolves — through UTC, not through the
    // naive local.
    props["start_utc"] = json!("2026-08-11T07:00:00Z");
    props["end_utc"] = json!("2026-08-11T07:15:00Z");
    let payload = payload_of(props, "create").await;
    assert_eq!(payload["start"]["timeZone"], json!("UTC"));
    assert_eq!(payload["start"]["dateTime"], json!("2026-08-11T07:00:00"));
}

/// Nothing to say is `null`, not `{}`. An empty PATCH still bumps the event's
/// change key at Graph, invalidating every stored `__etag` and making the next
/// delta re-deliver the whole series for no reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_with_no_writable_field_emits_null_not_an_empty_patch() {
    // `url` is not writable at all, so the allow-list intersects to nothing.
    let out = to_external(timed_event(), json!(["url"]), "update").await;
    assert!(out.is_none());
}

// ---------------------------------------------------------------------------
// mapper: the allow-list

/// `fields` is the engine's intersection of the mount's `mutable_fields` with
/// the adapter's, and emitting outside it is how a mount configured to push
/// titles quietly starts overwriting attendee lists.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_field_allow_list_is_honoured() {
    let mut props = timed_event();
    props["attendees"] = json!([{ "email": "a@example.com", "name": "A", "type": "required" }]);
    props["location"] = json!("Room 1");

    let payload = to_external(props, json!(["title"]), "update")
        .await
        .expect("title is writable")["payload"]
        .clone();
    assert_eq!(payload["subject"], json!("Standup"));
    assert!(payload.get("attendees").is_none(), "{payload:#?}");
    assert!(payload.get("location").is_none(), "{payload:#?}");
    assert!(payload.get("start").is_none(), "{payload:#?}");
}

/// The ONE deliberate exception to the allow-list: a time field never travels
/// alone.
///
/// Graph validates start against end on the same request, so a start that now
/// falls after the stored end is rejected outright, and a start sent without its
/// `timeZone` is read in whatever zone Graph last stored — which silently moves
/// the meeting. Every key in the group is written from this node's own
/// properties, so the extra ones carry what Graph already has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_time_field_drags_the_whole_time_group_with_it() {
    let payload = to_external(timed_event(), json!(["start_local"]), "update")
        .await
        .expect("start is writable")["payload"]
        .clone();
    assert_eq!(payload["start"]["dateTime"], json!("2026-08-11T09:00:00"));
    assert_eq!(payload["start"]["timeZone"], json!("Europe/Berlin"));
    assert_eq!(
        payload["end"]["dateTime"],
        json!("2026-08-11T09:15:00"),
        "an end must accompany a start or Graph validates against a stale one"
    );
    assert_eq!(payload["isAllDay"], json!(false));
    assert!(payload.get("subject").is_none(), "{payload:#?}");
}

/// An all-day event is a date, not an instant, and Graph wants midnight local.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_all_day_event_is_sent_as_local_midnight() {
    let mut props = timed_event();
    props["all_day"] = json!(true);
    props["start_local"] = json!("2026-08-11");
    props["end_local"] = json!("2026-08-12");
    let payload = payload_of(props, "create").await;
    assert_eq!(payload["isAllDay"], json!(true));
    assert_eq!(payload["start"]["dateTime"], json!("2026-08-11T00:00:00"));
    assert_eq!(payload["end"]["dateTime"], json!("2026-08-12T00:00:00"));
}

/// html wins when both bodies are present: Graph stores ONE body and text would
/// discard formatting the user can see.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn html_wins_over_text_and_the_body_type_travels_with_it() {
    let mut props = timed_event();
    props["description_text"] = json!("plain");
    props["description_html"] = json!("<p>rich</p>");
    let payload = payload_of(props.clone(), "create").await;
    assert_eq!(payload["body"], json!({"contentType":"html","content":"<p>rich</p>"}));

    props["description_html"] = Value::Null;
    let payload = payload_of(props, "create").await;
    assert_eq!(payload["body"], json!({"contentType":"text","content":"plain"}));
}

/// An empty attendee list CLEARS the invitees and is therefore emitted, unlike
/// an absent property. Dropping it would make "remove everyone" a silent no-op.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attendees_map_to_graph_shape_and_an_empty_list_is_meaningful() {
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
            { "emailAddress": { "address": "a@example.com", "name": "A" }, "type": "required" },
            { "emailAddress": { "address": "b@example.com" }, "type": "optional" },
        ]),
        "an attendee with no address cannot be invited and is dropped"
    );

    props["attendees"] = json!([]);
    assert_eq!(payload_of(props, "create").await["attendees"], json!([]));
}

// ---------------------------------------------------------------------------
// mapper: recurrence, the round trip

async fn pattern_of(rrule: &str, start: &str) -> Value {
    let mut props = timed_event();
    props["recurrence_type"] = json!("series_master");
    props["recurrence"] = json!([rrule]);
    props["start_local"] = json!(start);
    props["end_local"] = json!(start);
    payload_of(props, "update").await["recurrence"].clone()
}

/// Every Graph pattern type, reached from the RFC 5545 line the adapter's read
/// path emits for it. These two conversions are inverses and must stay so — the
/// read side is covered in `tests_ms_graph_calendar`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rfc5545_becomes_graph_patterned_recurrence() {
    let start = "2026-08-11T09:00:00"; // a Tuesday

    assert_eq!(
        pattern_of("RRULE:FREQ=DAILY;INTERVAL=3", start).await["pattern"],
        json!({ "type": "daily", "interval": 3 })
    );
    assert_eq!(
        pattern_of("RRULE:FREQ=WEEKLY;BYDAY=TU,TH", start).await["pattern"],
        json!({ "type": "weekly", "interval": 1, "daysOfWeek": ["tuesday", "thursday"] })
    );
    // A WEEKLY rule with no BYDAY repeats on the START's weekday. Graph demands
    // daysOfWeek, so it is derived rather than omitted.
    assert_eq!(
        pattern_of("RRULE:FREQ=WEEKLY", start).await["pattern"]["daysOfWeek"],
        json!(["tuesday"])
    );
    assert_eq!(
        pattern_of("RRULE:FREQ=MONTHLY;BYMONTHDAY=11", start).await["pattern"],
        json!({ "type": "absoluteMonthly", "interval": 1, "dayOfMonth": 11 })
    );
    assert_eq!(
        pattern_of("RRULE:FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2", start).await["pattern"],
        json!({ "type": "relativeMonthly", "interval": 1, "daysOfWeek": ["tuesday"], "index": "second" })
    );
    assert_eq!(
        pattern_of("RRULE:FREQ=YEARLY;BYMONTH=8;BYMONTHDAY=11", start).await["pattern"],
        json!({ "type": "absoluteYearly", "interval": 1, "dayOfMonth": 11, "month": 8 })
    );
    assert_eq!(
        pattern_of("RRULE:FREQ=YEARLY;BYMONTH=8;BYDAY=TU;BYSETPOS=-1", start).await["pattern"],
        json!({ "type": "relativeYearly", "interval": 1, "daysOfWeek": ["tuesday"], "index": "last", "month": 8 })
    );
}

/// The UNTIL/endDate inverse, which is the one arithmetic error that COMPOUNDS.
///
/// The adapter reads Graph's `endDate` — a date in the series' own zone — into a
/// UTC UNTIL instant by taking the local time of day and padding 12 hours, so
/// that a series whose local time falls on the next UTC day keeps its final
/// occurrence. Writing back has to subtract the same 12 hours before taking the
/// date, or every save moves the end date one day later and the series grows an
/// occurrence per save, forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn until_round_trips_back_to_the_same_end_date() {
    // 09:00 local + 12h = 21:00Z on the same day — what the read path emits.
    let range = pattern_of("RRULE:FREQ=DAILY;UNTIL=20261231T210000Z", "2026-08-11T09:00:00").await
        ["range"]
        .clone();
    assert_eq!(range["type"], json!("endDate"));
    assert_eq!(range["endDate"], json!("2026-12-31"));

    // A 19:00 series: 19:00 + 12h = 07:00Z on 2027-01-01. Taking the date off
    // that instant would say 2027-01-01 and add a day per save.
    let range = pattern_of("RRULE:FREQ=DAILY;UNTIL=20270101T070000Z", "2026-08-11T19:00:00").await
        ["range"]
        .clone();
    assert_eq!(range["endDate"], json!("2026-12-31"));

    // A foreign RRULE that wrote a naive end-of-day still lands on its own date.
    let range = pattern_of("RRULE:FREQ=DAILY;UNTIL=20261231T235959Z", "2026-08-11T09:00:00").await
        ["range"]
        .clone();
    assert_eq!(range["endDate"], json!("2026-12-31"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn count_and_unbounded_ranges_map_to_their_graph_forms() {
    let start = "2026-08-11T09:00:00";
    let range = pattern_of("RRULE:FREQ=DAILY;COUNT=10", start).await["range"].clone();
    assert_eq!(range["type"], json!("numbered"));
    assert_eq!(range["numberOfOccurrences"], json!(10));
    assert_eq!(range["startDate"], json!("2026-08-11"));
    // The zone rides along, because Graph expands the rule server-side and a
    // range with no zone is expanded in the mailbox's, not the series'.
    assert_eq!(range["recurrenceTimeZone"], json!("Europe/Berlin"));

    assert_eq!(
        pattern_of("RRULE:FREQ=DAILY", start).await["range"]["type"],
        json!("noEnd")
    );
}

/// Graph's pattern vocabulary is strictly narrower than RRULE's. A rule it
/// cannot express is DROPPED rather than approximated — a "close enough" pattern
/// silently reschedules a real series, which is worse than not writing the rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rule_graph_cannot_express_is_dropped_not_approximated() {
    let mut props = timed_event();
    props["recurrence_type"] = json!("series_master");
    props["recurrence"] = json!(["RRULE:FREQ=HOURLY;INTERVAL=6"]);
    let payload = payload_of(props, "update").await;
    assert!(payload.get("recurrence").is_none(), "{payload:#?}");
    // The rest of the event still travels: the rule is unrepresentable, not the
    // event.
    assert_eq!(payload["subject"], json!("Standup"));
}

// ---------------------------------------------------------------------------
// adapter: capabilities

/// The headline of this change. Before it, ms-graph declared calendar
/// `can_write` for RSVP alone, so every calendar MIRROR mount resolved to
/// `Refused` — the write path's own use case was unreachable against this
/// provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_calendar_mount_now_declares_the_full_mirror_surface() {
    let run = call_adapter(
        json!({ "operation": "capabilities", "mount": calendar_mount(false) }),
        vec![],
    )
    .await;
    let caps = run.output.expect("capabilities");
    assert_eq!(caps["can_write"], json!(true));
    assert_eq!(caps["can_create"], json!(true));
    assert_eq!(caps["can_update"], json!(true));
    assert_eq!(caps["can_delete"], json!(true));
    assert_eq!(caps["can_submit"], json!(true), "RSVP is unaffected");
    assert_eq!(caps["supports_trash"], json!(true));
    assert_eq!(caps["default_delete_policy"], json!("trash"));

    // NODE property names, and only the ones the mapper can actually emit. A
    // name here the mapper ignores is a field the engine plans pushes for and
    // that never converges.
    let fields = caps["mutable_fields"].as_array().expect("mutable_fields");
    for f in ["title", "start_local", "attendees", "recurrence"] {
        assert!(fields.iter().any(|v| v == f), "{fields:#?} is missing {f}");
    }
    // Everything Graph owns stays out: cancelling is a DELETE under the mount's
    // delete_policy and an RSVP is a `submit` command, deliberately, because it
    // notifies the organizer and must not hide behind a property edit.
    for f in ["status", "my_response", "organizer_email", "ical_uid", "url"] {
        assert!(
            !fields.iter().any(|v| v == f),
            "{f} is not writable at Graph but is declared: {fields:#?}"
        );
    }
}

// ---------------------------------------------------------------------------
// adapter: the calls

fn write_input(op: &str, params: Value) -> Value {
    json!({
        "operation": op,
        "credential": { "access_token": "TOKEN123", "provider_type": "ms-graph" },
        "mount": calendar_mount(false),
        "params": params,
    })
}

/// A create POSTs to the MOUNT's calendar, not to `/me/events`.
///
/// The distinction is the same one the read path makes and for the same reason:
/// a default `/events` write would land in the CONNECTED account's primary
/// calendar while the mount reads a shared or secondary one — silently, with no
/// error anywhere and no way to notice except by looking at the wrong calendar.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_posts_into_the_mounts_own_calendar_and_returns_the_new_id() {
    let run = call_adapter(
        write_input(
            "create",
            json!({ "payload": { "subject": "Standup" }, "parent_id": "AAMkCal==" }),
        ),
        vec![json!({
            "status": 201,
            "headers": {},
            "body": { "id": "EV1", "@odata.etag": "W/\"v1\"" }
        })],
    )
    .await;

    let call = &run.calls[0];
    assert_eq!(call["method"], json!("POST"));
    assert!(
        call["url"].as_str().unwrap().contains("/calendars/AAMkCal%3D%3D/events"),
        "{}",
        call["url"]
    );
    let out = run.output.expect("create output");
    assert_eq!(out["external_id"], json!("EV1"));
    assert_eq!(out["etag"], json!("W/\"v1\""));
}

/// An update PATCHes the EVENT route, which is what the whole non-mail refusal
/// was blocking. The stored etag becomes If-Match only when it has the shape of
/// one — `toExternalItem` falls back to `lastModifiedDateTime`, and sending that
/// is a 400, which is terminal and would mark the mount misconfigured.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_patches_the_event_and_sends_if_match_only_for_a_real_etag() {
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "subject": "Renamed" }, "etag": "W/\"v1\"" }),
        ),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "EV1", "@odata.etag": "W/\"v2\"" } })],
    )
    .await;
    let call = &run.calls[0];
    assert_eq!(call["method"], json!("PATCH"));
    assert!(call["url"].as_str().unwrap().contains("/events/EV1"), "{}", call["url"]);
    assert_eq!(call["options"]["headers"]["If-Match"], json!("W/\"v1\""));
    assert_eq!(run.output.expect("output")["etag"], json!("W/\"v2\""));

    // An ISO timestamp in `__etag` is a healthy node, not a broken one.
    let run = call_adapter(
        write_input(
            "update",
            json!({ "item_id": "EV1", "payload": { "subject": "x" }, "etag": "2026-08-11T09:00:00Z" }),
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

/// A 404 on an update SETTLES the node rather than failing it: Graph ids are not
/// stable, and the delta feed re-imports the item under its new id.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_against_a_vanished_event_settles_rather_than_failing() {
    let run = call_adapter(
        write_input("update", json!({ "item_id": "EV1", "payload": { "subject": "x" } })),
        vec![json!({ "status": 404, "headers": {}, "body": { "error": { "code": "ErrorItemNotFound" } } })],
    )
    .await;
    assert_eq!(run.output, Some(Value::Null));
}

/// A delete against an event that is ALREADY GONE is success. It is the one
/// operation whose desired end state a 404 already satisfies, and failing it
/// would leave the engine retrying forever against an id that cannot come back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delete_is_idempotent_and_soft() {
    let run = call_adapter(
        write_input("delete", json!({ "item_id": "EV1", "policy": "trash", "etag": "W/\"v1\"" })),
        vec![json!({ "status": 204, "headers": {}, "body": Value::Null })],
    )
    .await;
    assert_eq!(run.calls[0]["method"], json!("DELETE"));
    assert_eq!(run.output.expect("output")["deleted"], json!(true));

    let run = call_adapter(
        write_input("delete", json!({ "item_id": "EV1", "policy": "trash" })),
        vec![json!({ "status": 404, "headers": {}, "body": { "error": { "code": "ErrorItemNotFound" } } })],
    )
    .await;
    assert_eq!(run.output.expect("output")["deleted"], json!(true));
}

/// `purge` is REFUSED, not quietly served as a soft delete.
///
/// Graph's DELETE moves an event to Deleted Items and Graph offers no permanent
/// delete for one at all. `purge` is the single policy the engine will never
/// default to — an operator has to type it — so answering "done" to a request for
/// irreversibility we cannot perform is the failure that matters here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn purge_is_refused_because_graph_cannot_do_it() {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(write_input(
        "delete",
        json!({ "item_id": "EV1", "policy": "purge" }),
    ));
    let result = runtime
        .execute(
            &adapter_source(),
            "handler",
            context,
            &FunctionMetadata::javascript("ms_graph_adapter"),
            api.clone() as Arc<dyn crate::api::FunctionApi>,
            adapter_files(),
        )
        .await
        .expect("runtime execute");
    let err = result.error.expect("purge must throw");
    assert!(err.message.contains("config_error"), "{}", err.message);
    assert!(
        err.message.contains("delete_policy"),
        "the message must name the fix: {}",
        err.message
    );
    assert!(api.http_calls().is_empty(), "nothing may reach Graph");
}
