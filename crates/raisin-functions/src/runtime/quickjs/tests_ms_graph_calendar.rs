// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The ms-graph adapter's CALENDAR read path, executed as the real file in
//! QuickJS with `raisin.http.fetch` scripted by [`MockFunctionApi`].
//!
//! The recurrence conversion is the reason this file exists: `patternedRecurrence`
//! is a nested Graph-specific object and `raisin:Event.recurrence` is an array of
//! RFC 5545 content lines, so every pattern type and every range type has an exact
//! expected string here. A mapper-level test cannot cover it — the conversion now
//! lives in the adapter so that Graph and Google hand the engine the same shape.

use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

use super::tests_ms_graph_adapter::{adapter_files, adapter_source};

pub(super) struct Run {
    pub(super) output: Option<Value>,
    pub(super) calls: Vec<Value>,
}

pub(super) async fn call_adapter(input: Value, responses: Vec<Value>) -> Run {
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
    if let Some(err) = result.error {
        panic!("adapter threw: {}", err.message);
    }
    Run {
        output: result.output,
        calls: api.http_calls(),
    }
}

pub(super) fn calendar_mount(include_body: bool) -> Value {
    let mut config = json!({ "resource": "calendar" });
    if include_body {
        config["include_body"] = json!(true);
    }
    json!({
        "mount_id": "m1",
        "remote_root": "calendar",
        "config": config,
        "sync_config": config,
    })
}

pub(super) fn list_input() -> Value {
    json!({
        "operation": "list",
        "credential": { "access_token": "TOKEN123", "provider_type": "ms-graph" },
        "mount": calendar_mount(false),
        "params": { "limit": 50 },
    })
}

pub(super) fn page(values: Vec<Value>) -> Value {
    json!({ "status": 200, "headers": {}, "body": { "value": values } })
}

/// One `list` call over a single seriesMaster carrying `pattern`/`range`, and
/// the resulting `metadata.recurrence`. A master triggers the `/instances`
/// expansion, so a second (empty) response is scripted for it.
async fn rrule_of(pattern: Value, range: Value) -> Value {
    let master = json!({
        "id": "S1",
        "type": "seriesMaster",
        "subject": "Standup",
        "start": { "dateTime": "2026-08-11T09:00:00.0000000", "timeZone": "W. Europe Standard Time" },
        "end": { "dateTime": "2026-08-11T09:15:00.0000000", "timeZone": "W. Europe Standard Time" },
        "recurrence": { "pattern": pattern, "range": range },
    });
    let run = call_adapter(list_input(), vec![page(vec![master]), page(vec![])]).await;
    run.output.expect("list output")["items"][0]["metadata"]["recurrence"].clone()
}

// ---------------------------------------------------------------------------

/// Every Graph pattern type and every range type, with the exact RFC 5545 line.
/// Before this stage the adapter emitted `JSON.stringify(v.recurrence)` — a Graph
/// JSON blob in a column the nodetype documents as an RRULE, and a shape the
/// Google side could never produce.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patterned_recurrence_becomes_rfc5545_lines() {
    let no_end = json!({ "type": "noEnd", "startDate": "2026-08-11" });

    // daily, interval 1 -> INTERVAL omitted (RFC default).
    assert_eq!(
        rrule_of(json!({ "type": "daily", "interval": 1 }), no_end.clone()).await,
        json!(["RRULE:FREQ=DAILY"])
    );
    assert_eq!(
        rrule_of(json!({ "type": "daily", "interval": 3 }), no_end.clone()).await,
        json!(["RRULE:FREQ=DAILY;INTERVAL=3"])
    );

    // weekly + byDay, order preserved from Graph's own list.
    assert_eq!(
        rrule_of(
            json!({
                "type": "weekly",
                "interval": 2,
                "daysOfWeek": ["monday", "wednesday", "friday"],
                "firstDayOfWeek": "sunday",
            }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE,FR;WKST=SU"])
    );
    // firstDayOfWeek=monday IS the RFC default, so no WKST is emitted — two
    // equivalent rules must not compare unequal.
    assert_eq!(
        rrule_of(
            json!({ "type": "weekly", "interval": 1, "daysOfWeek": ["tuesday"], "firstDayOfWeek": "monday" }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=WEEKLY;BYDAY=TU"])
    );

    // absoluteMonthly -> BYMONTHDAY.
    assert_eq!(
        rrule_of(
            json!({ "type": "absoluteMonthly", "interval": 1, "dayOfMonth": 15 }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=MONTHLY;BYMONTHDAY=15"])
    );

    // relativeMonthly -> BYDAY + BYSETPOS. "last" is -1, not 5.
    assert_eq!(
        rrule_of(
            json!({
                "type": "relativeMonthly",
                "interval": 1,
                "daysOfWeek": ["thursday"],
                "index": "last",
            }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=MONTHLY;BYDAY=TH;BYSETPOS=-1"])
    );
    assert_eq!(
        rrule_of(
            json!({
                "type": "relativeMonthly",
                "interval": 2,
                "daysOfWeek": ["monday"],
                "index": "second",
            }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=MONTHLY;INTERVAL=2;BYDAY=MO;BYSETPOS=2"])
    );

    // absoluteYearly -> BYMONTH + BYMONTHDAY, in that order.
    assert_eq!(
        rrule_of(
            json!({ "type": "absoluteYearly", "interval": 1, "month": 12, "dayOfMonth": 24 }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=YEARLY;BYMONTH=12;BYMONTHDAY=24"])
    );

    // relativeYearly -> BYMONTH + BYDAY + BYSETPOS.
    assert_eq!(
        rrule_of(
            json!({
                "type": "relativeYearly",
                "interval": 1,
                "month": 11,
                "daysOfWeek": ["thursday"],
                "index": "fourth",
            }),
            no_end.clone(),
        )
        .await,
        json!(["RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=TH;BYSETPOS=4"])
    );

    // Ranges. noEnd emits nothing (unbounded is the RFC default); endDate is
    // INCLUSIVE on both sides.
    //
    // NOT `T235959Z`. Graph's endDate is a date in the SERIES' OWN ZONE, while
    // RFC 5545 compares UNTIL against the instants the rule expands to, so a
    // naive end-of-UTC-day cutoff silently drops the last occurrence of every
    // series whose local time falls on the next UTC day (19:00 America/New_York
    // on 2026-12-31 is 2027-01-01T00:00:00Z). The sandbox has no tz database,
    // so `graphUntil` instead takes the series' local time-of-day on endDate,
    // reads it as UTC and adds 12h — a cutoff that lies strictly between the
    // last occurrence and the next one for every zone down to UTC-12. Here
    // 09:00 + 12h = 21:00Z. Asserting the exact instant, because the padding IS
    // the contract; loosening this to "starts with 20261231" would let the
    // Americas regression back in unnoticed.
    assert_eq!(
        rrule_of(
            json!({ "type": "daily", "interval": 1 }),
            json!({ "type": "endDate", "startDate": "2026-08-11", "endDate": "2026-12-31" }),
        )
        .await,
        json!(["RRULE:FREQ=DAILY;UNTIL=20261231T210000Z"])
    );
    assert_eq!(
        rrule_of(
            json!({ "type": "daily", "interval": 1 }),
            json!({ "type": "numbered", "startDate": "2026-08-11", "numberOfOccurrences": 10 }),
        )
        .await,
        json!(["RRULE:FREQ=DAILY;COUNT=10"])
    );

    // An unknown pattern type degrades to null rather than emitting a rule that
    // expands to the wrong instants.
    assert_eq!(
        rrule_of(json!({ "type": "somethingNew", "interval": 1 }), no_end).await,
        Value::Null
    );
}
