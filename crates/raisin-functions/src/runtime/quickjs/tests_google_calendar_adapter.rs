// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The google-calendar adapter, executed as the real file in QuickJS with
//! `raisin.http.fetch` scripted by [`MockFunctionApi`].
//!
//! Google needs no recurrence conversion — `recurrence` is already an array of
//! RFC 5545 content lines. What it needed was to STOP HIDING it: every read path
//! sent `singleEvents=true`, so a master was never returned and the field was
//! dead. These tests pin the request shape (the parameter is what decides which
//! entities Google returns) and the metadata that used to be discarded.

use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

fn adapter_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../builtin-packages/google-calendar-adapter/content/functions/adapters/google-calendar",
    )
}

fn adapter_source() -> String {
    let path = adapter_dir().join("index.js");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read adapter at {}: {e}", path.display()))
}

/// The adapter's sibling MODULES, keyed exactly as the runtime keys them: the
/// path relative to the function node, which for a flat function directory is
/// the bare filename. Mirrors `load_sibling_files`, which lists the function
/// node's children in storage. Without it the entrypoint's imports resolve to
/// nothing — and the resolver REJECTS an unknown specifier rather than returning
/// undefined, so the failure is total rather than subtle.
fn adapter_files() -> HashMap<String, String> {
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

pub(super) struct Run {
    pub(super) output: Option<Value>,
    #[allow(dead_code)]
    pub(super) error: Option<String>,
    #[allow(dead_code)]
    pub(super) calls: Vec<Value>,
}

pub(super) async fn call_adapter(input: Value, responses: Vec<Value>) -> Run {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})).with_http_responses(responses));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(input);
    let metadata = FunctionMetadata::javascript("google_calendar_adapter");
    let result = runtime
        .execute(
            &FunctionCode::from(adapter_source()),
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

pub(super) fn mount(include_body: bool) -> Value {
    let mut sync = json!({});
    if include_body {
        sync["include_body"] = json!(true);
    }
    json!({ "mount_id": "m1", "remote_root": "primary", "sync_config": sync })
}

pub(super) fn list_input(include_body: bool) -> Value {
    json!({
        "operation": "list",
        "credential": { "access_token": "TOKEN123" },
        "mount": mount(include_body),
        "params": { "limit": 50 },
    })
}

pub(super) fn ok(items: Vec<Value>, extra: Value) -> Value {
    let mut body = json!({ "items": items });
    if let Some(map) = extra.as_object() {
        for (k, v) in map {
            body[k] = v.clone();
        }
    }
    json!({ "status": 200, "headers": {}, "body": body })
}

// ---------------------------------------------------------------------------

/// `singleEvents=true` on every read path meant a recurring MASTER was never
/// returned — Google expanded it into instances first — so `ev.recurrence` was
/// only ever present on an entity this adapter could not see. The mapper's
/// recurrence handling was therefore dead code. `orderBy=startTime` has to go
/// with it: Google accepts that ordering ONLY with singleEvents=true, so leaving
/// it behind would 400 every list.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_read_path_expands_occurrences_any_more() {
    let list = call_adapter(list_input(false), vec![ok(vec![], json!({}))]).await;
    let url = list.calls[0]["url"].as_str().unwrap();
    assert!(!url.contains("singleEvents"), "list still expands: {url}");
    assert!(
        !url.contains("orderBy"),
        "orderBy without singleEvents is a 400: {url}"
    );
    assert!(
        url.contains("timeMin=") && url.contains("timeMax="),
        "{url}"
    );

    // Baseline (no since_token) and incremental must agree with the list, or
    // Google rejects the token the baseline minted.
    let base = call_adapter(
        json!({
            "operation": "get_changes",
            "credential": { "access_token": "T" },
            "mount": mount(false),
            "params": {},
        }),
        vec![ok(vec![], json!({ "nextSyncToken": "TOK1" }))],
    )
    .await;
    assert!(!base.calls[0]["url"]
        .as_str()
        .unwrap()
        .contains("singleEvents"));
    assert_eq!(base.output.expect("baseline")["next_token"], json!("TOK1"));

    let incr = call_adapter(
        json!({
            "operation": "get_changes",
            "credential": { "access_token": "T" },
            "mount": mount(false),
            "params": { "since_token": "TOK1" },
        }),
        vec![ok(vec![], json!({ "nextSyncToken": "TOK2" }))],
    )
    .await;
    let iurl = incr.calls[0]["url"].as_str().unwrap();
    assert!(!iurl.contains("singleEvents"), "{iurl}");
    assert!(
        iurl.contains("syncToken=TOK1") && iurl.contains("showDeleted=true"),
        "{iurl}"
    );
}

/// A syncToken is only valid for a request with the SAME parameters that minted
/// it, so every token stored by the previous build is rejected once. Reporting
/// that 400 as `cursor_invalid` is what turns a breaking parameter change into a
/// single automatic full reconcile instead of a permanently stuck mount. The 410
/// beside it used to throw a PLAIN error, which the engine reads as transient —
/// it retried the same expired token on every tick, forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_minted_by_the_old_parameters_forces_one_full_reconcile() {
    for status in [400, 410] {
        let run = call_adapter(
            json!({
                "operation": "get_changes",
                "credential": { "access_token": "T" },
                "mount": mount(false),
                "params": { "since_token": "STALE" },
            }),
            vec![json!({
                "status": status,
                "headers": {},
                "body": { "error": { "message": "Sync token is no longer valid" } },
            })],
        )
        .await;
        let err = run.error.unwrap_or_else(|| panic!("{status} must throw"));
        assert!(err.contains("[code=cursor_invalid]"), "{status}: {err}");
        // AdapterError::classify scans auth_expired and rate_limited FIRST, so
        // either token in the message would misclassify this.
        assert!(!err.contains("auth_expired"), "{status}: {err}");
        assert!(!err.contains("rate_limited"), "{status}: {err}");
    }
}

/// The fields that were on the wire in every response and thrown away by the
/// 12-key whitelist. `recurringEventId` and `originalStartTime` are the pair
/// that links an exception back to its master; without them Google's instances
/// were mutually unrelated nodes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exception_carries_the_link_back_to_its_master() {
    let master = json!({
        "id": "S1",
        "iCalUID": "abc@google.com",
        "summary": "Weekly",
        "status": "confirmed",
        "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=TU", "EXDATE;TZID=Europe/Zurich:20260811T090000"],
        "start": { "dateTime": "2026-08-11T09:00:00+02:00", "timeZone": "Europe/Zurich" },
        "end": { "dateTime": "2026-08-11T09:30:00+02:00", "timeZone": "Europe/Zurich" },
    });
    let exception = json!({
        "id": "S1_20260818T070000Z",
        "recurringEventId": "S1",
        "originalStartTime": { "dateTime": "2026-08-18T09:00:00+02:00", "timeZone": "Europe/Zurich" },
        "summary": "Weekly (moved)",
        "status": "confirmed",
        "start": { "dateTime": "2026-08-18T10:00:00+02:00", "timeZone": "Europe/Zurich" },
        "end": { "dateTime": "2026-08-18T10:30:00+02:00", "timeZone": "Europe/Zurich" },
    });

    let run = call_adapter(
        list_input(false),
        vec![ok(vec![master, exception], json!({}))],
    )
    .await;
    let items = run.output.expect("list output")["items"].clone();

    let m = &items[0]["metadata"];
    assert_eq!(
        m["recurrence"],
        json!([
            "RRULE:FREQ=WEEKLY;BYDAY=TU",
            "EXDATE;TZID=Europe/Zurich:20260811T090000"
        ])
    );
    assert_eq!(m["recurring_event_id"], Value::Null);
    assert_eq!(m["ical_uid"], json!("abc@google.com"));
    // The IANA zone Google sends alongside every slot, never read before.
    assert_eq!(m["start_timezone"], json!("Europe/Zurich"));

    let e = &items[1]["metadata"];
    assert_eq!(e["recurring_event_id"], json!("S1"));
    assert_eq!(
        e["original_start"]["dateTime"],
        json!("2026-08-18T09:00:00+02:00")
    );
    assert_eq!(e["recurrence"], Value::Null);
}

/// A NON-RECURRING event is the regression risk of this stage: it must still map
/// and must gain the newly carried fields without losing any old one. Organizer
/// is now the RAW object — reduced to a bare email before, which discarded the
/// `self` flag the RSVP is read from and disagreed with the Graph side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_event_still_maps_and_the_body_stays_opt_in() {
    let ev = json!({
        "id": "E1",
        "summary": "Lunch",
        "status": "confirmed",
        "location": "Kantine",
        "htmlLink": "https://calendar.example/E1",
        "etag": "\"123\"",
        "transparency": "transparent",
        "hangoutLink": "https://meet.example/xyz",
        "description": "<p>hi</p>",
        "organizer": { "email": "ada@example.com", "displayName": "Ada", "self": true },
        "attendees": [{ "email": "bob@example.com", "responseStatus": "declined", "optional": true }],
        "start": { "dateTime": "2026-08-11T12:00:00+02:00", "timeZone": "Europe/Zurich" },
        "end": { "dateTime": "2026-08-11T13:00:00+02:00", "timeZone": "Europe/Zurich" },
    });

    let run = call_adapter(list_input(false), vec![ok(vec![ev.clone()], json!({}))]).await;
    let item = run.output.expect("list output")["items"][0].clone();
    assert_eq!(item["external_id"], json!("E1"));
    assert_eq!(item["etag"], json!("\"123\""));
    let meta = &item["metadata"];
    assert_eq!(meta["summary"], json!("Lunch"));
    assert_eq!(meta["location"], json!("Kantine"));
    assert_eq!(meta["calendar_id"], json!("primary"));
    assert_eq!(meta["recurrence"], Value::Null);
    assert_eq!(meta["recurring_event_id"], Value::Null);
    assert_eq!(meta["transparency"], json!("transparent"));
    assert_eq!(
        meta["online_meeting_url"],
        json!("https://meet.example/xyz")
    );
    // The raw organizer, so `self` survives to drive my_response.
    assert_eq!(meta["organizer"]["self"], json!(true));
    assert_eq!(meta["organizer"]["displayName"], json!("Ada"));
    // Absent, not "": an empty description would blank a synced one every run.
    assert!(meta.get("description").is_none(), "description leaked");

    let opted = call_adapter(list_input(true), vec![ok(vec![ev], json!({}))]).await;
    assert_eq!(
        opted.output.expect("opted output")["items"][0]["metadata"]["description"],
        json!("<p>hi</p>")
    );
}
