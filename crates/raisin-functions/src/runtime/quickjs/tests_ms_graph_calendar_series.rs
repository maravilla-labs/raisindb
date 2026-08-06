// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The ms-graph adapter's CALENDAR series/exception behaviour and its field
//! projection, executed as the real file in QuickJS. Shares the harness with
//! [`super::tests_ms_graph_calendar`]; the recurrence CONVERSION lives there.

use super::tests_ms_graph_calendar::{calendar_mount, call_adapter, list_input, page};
use serde_json::{json, Value};

/// A NON-RECURRING event is the regression risk of this whole stage: it must
/// still map, must not trigger the `/instances` expansion, and must carry the
/// fields that used to be fetched and dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_event_still_maps_and_costs_one_request() {
    let ev = json!({
        "id": "E1",
        "type": "singleInstance",
        "iCalUId": "040000008200E000",
        "subject": "Lunch",
        "start": { "dateTime": "2026-08-11T12:00:00.0000000", "timeZone": "W. Europe Standard Time" },
        "end": { "dateTime": "2026-08-11T13:00:00.0000000", "timeZone": "W. Europe Standard Time" },
        "isAllDay": false,
        "isCancelled": false,
        "showAs": "busy",
        "responseStatus": { "response": "accepted" },
        "location": {
            "displayName": "Kantine",
            "coordinates": { "latitude": 47.3769, "longitude": 8.5417 },
        },
        "organizer": { "emailAddress": { "name": "Ada", "address": "ada@contoso.com" } },
        "attendees": [{
            "type": "optional",
            "status": { "response": "declined" },
            "emailAddress": { "name": "Bob", "address": "bob@contoso.com" },
        }],
        "onlineMeeting": { "joinUrl": "https://teams.example/join" },
        "webLink": "https://outlook.example/E1",
        "body": { "contentType": "html", "content": "<p>hi</p>" },
    });

    let run = call_adapter(list_input(), vec![page(vec![ev])]).await;
    // A single instance is not a series: no `/instances` expansion.
    assert_eq!(
        run.calls.len(),
        1,
        "a non-recurring page made extra requests"
    );

    let meta = run.output.expect("list output")["items"][0]["metadata"].clone();
    assert_eq!(meta["recurrence"], Value::Null);
    assert_eq!(meta["event_type"], json!("singleInstance"));
    assert_eq!(meta["series_master_id"], Value::Null);
    assert_eq!(meta["ical_uid"], json!("040000008200E000"));

    // The three concepts that used to be folded into one `status` string.
    assert_eq!(meta["is_cancelled"], json!(false));
    assert_eq!(meta["show_as"], json!("busy"));
    assert_eq!(meta["my_response"], json!("accepted"));
    assert!(
        meta.get("status").is_none(),
        "the conflated column survived"
    );

    // Timezone: captured before this stage, then dropped by the mapper.
    assert_eq!(meta["start_tz"], json!("W. Europe Standard Time"));

    // Attendees are the RAW objects, so response and type survive; the old
    // "Name <addr>" flattening discarded both and disagreed with Google.
    assert_eq!(
        meta["attendees"][0]["status"]["response"],
        json!("declined")
    );
    assert_eq!(meta["attendees"][0]["type"], json!("optional"));
    assert_eq!(
        meta["organizer"]["emailAddress"]["address"],
        json!("ada@contoso.com")
    );

    // GeoJSON is [lon, lat], never [lat, lon].
    assert_eq!(
        meta["location_geo"],
        json!({ "type": "Point", "coordinates": [8.5417, 47.3769] })
    );
    assert_eq!(
        meta["online_meeting_url"],
        json!("https://teams.example/join")
    );

    // The body is opt-in; this mount did not ask, so the key is ABSENT (not "")
    // — an empty string would blank a previously synced description every run.
    assert!(
        meta.get("body").is_none(),
        "body leaked without include_body"
    );
}

/// `$select` is now sent on the calendar list, the single get, the delta feed
/// and the series-master resolution. The delta feed and the get sent none at
/// all, which is what made `responseStatus` path-dependent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_calendar_read_path_projects_the_same_fields() {
    let run = call_adapter(list_input(), vec![page(vec![])]).await;
    let list_url = run.calls[0]["url"].as_str().unwrap().to_string();
    assert!(list_url.contains("responseStatus"), "{list_url}");
    assert!(list_url.contains("iCalUId"), "{list_url}");
    assert!(list_url.contains("originalStart"), "{list_url}");
    assert!(
        !list_url.contains("body"),
        "body in a mount that did not opt in"
    );

    let get = call_adapter(
        json!({
            "operation": "get",
            "credential": { "access_token": "T" },
            "mount": calendar_mount(false),
            "params": { "item_id": "E1" },
        }),
        vec![json!({ "status": 200, "headers": {}, "body": { "id": "E1" } })],
    )
    .await;
    assert!(
        get.calls[0]["url"].as_str().unwrap().contains("$select="),
        "get sends no projection: {}",
        get.calls[0]["url"]
    );

    let changes = call_adapter(
        json!({
            "operation": "get_changes",
            "credential": { "access_token": "T" },
            "mount": calendar_mount(false),
            "params": {},
        }),
        vec![json!({ "status": 200, "headers": {}, "body": { "value": [] } })],
    )
    .await;
    let delta_url = changes.calls[0]["url"].as_str().unwrap().to_string();
    assert!(delta_url.contains("calendarView/delta"), "{delta_url}");
    assert!(
        delta_url.contains("$select="),
        "delta sends no projection: {delta_url}"
    );

    // include_body widens exactly one thing and only for the opted-in mount.
    let mut with_body = list_input();
    with_body["mount"] = calendar_mount(true);
    let opted = call_adapter(with_body, vec![page(vec![])]).await;
    assert!(opted.calls[0]["url"].as_str().unwrap().contains("body"));
}

/// An EXCEPTION is a node of its own. The delta path folded every occurrence AND
/// every exception into the series master, so rescheduling one occurrence of a
/// weekly meeting produced an "update" whose properties were byte-identical —
/// the override was invisible in the data. A plain occurrence still collapses:
/// it adds nothing the rule does not already say.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_delta_emits_an_exception_alongside_its_master_but_not_an_occurrence() {
    let master = json!({
        "id": "S1",
        "type": "seriesMaster",
        "subject": "Weekly",
        "recurrence": {
            "pattern": { "type": "weekly", "interval": 1, "daysOfWeek": ["tuesday"] },
            "range": { "type": "noEnd" },
        },
    });
    let occurrence = json!({ "id": "S1_OCC1", "type": "occurrence", "seriesMasterId": "S1" });
    let exception = json!({
        "id": "S1_EXC1",
        "type": "exception",
        "seriesMasterId": "S1",
        "subject": "Weekly (moved)",
        "originalStart": "2026-08-11T07:00:00Z",
        "start": { "dateTime": "2026-08-12T09:00:00.0000000", "timeZone": "UTC" },
    });

    let run = call_adapter(
        json!({
            "operation": "get_changes",
            "credential": { "access_token": "T" },
            "mount": calendar_mount(false),
            "params": { "since_token": "https://graph.microsoft.com/v1.0/me/calendarView/delta?$deltatoken=x" },
        }),
        vec![json!({
            "status": 200,
            "headers": {},
            "body": { "value": [master, occurrence, exception], "@odata.deltaLink": "LINK" },
        })],
    )
    .await;

    let items = run.output.expect("get_changes output")["items"].clone();
    let ids: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item"]["external_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["S1", "S1_EXC1"], "wrong item set: {items}");

    // The master carries the rule; the exception carries the slot it replaces.
    assert_eq!(
        items[0]["item"]["metadata"]["recurrence"],
        json!(["RRULE:FREQ=WEEKLY;BYDAY=TU"])
    );
    let exc = &items[1]["item"]["metadata"];
    assert_eq!(exc["event_type"], json!("exception"));
    assert_eq!(exc["series_master_id"], json!("S1"));
    assert_eq!(exc["original_start"], json!("2026-08-11T07:00:00Z"));
    // The page already held the master, so no extra fetch was needed.
    assert_eq!(run.calls.len(), 1, "an in-page master was re-fetched");
}

/// The full walk must emit the SAME set as the delta, or a reconcile finds the
/// delta-created exception nodes absent from the listing and deletes them —
/// they would flap in and out of existence on alternating runs. `/events`
/// returns masters and single instances only, so each master is expanded once
/// through `/instances`, plus one `cancelledOccurrences` probe.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_full_walk_expands_each_master_for_its_exceptions() {
    let master = json!({
        "id": "S1",
        "type": "seriesMaster",
        "subject": "Weekly",
        "recurrence": {
            "pattern": { "type": "weekly", "interval": 1, "daysOfWeek": ["tuesday"] },
            "range": { "type": "noEnd" },
        },
    });
    let instances = page(vec![
        json!({ "id": "S1_OCC1", "type": "occurrence", "seriesMasterId": "S1" }),
        json!({
            "id": "S1_EXC1",
            "type": "exception",
            "seriesMasterId": "S1",
            "originalStart": "2026-08-11T07:00:00Z",
        }),
    ]);

    let run = call_adapter(list_input(), vec![page(vec![master]), instances]).await;
    let items = run.output.expect("list output")["items"].clone();
    let ids: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["external_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["S1", "S1_EXC1"], "wrong item set: {items}");

    // Two requests PER MASTER, not one: `/instances` for exceptions, then the
    // `cancelledOccurrences` probe for the slots Graph deletes outright and
    // reports nowhere else. The probe is what keeps a cancelled standup from
    // being regenerated by every expander rebuild, so it is part of the walk's
    // contract — assert it is issued, and that neither is issued twice.
    let urls: Vec<&str> = run
        .calls
        .iter()
        .map(|c| c["url"].as_str().unwrap())
        .collect();
    assert_eq!(urls.len(), 3, "calls were: {urls:#?}");
    assert!(
        urls[2].contains("cancelledOccurrences"),
        "no cancelled-occurrence probe: {urls:#?}"
    );
    let inst_url = run.calls[1]["url"].as_str().unwrap();
    assert!(inst_url.contains("/events/S1/instances"), "{inst_url}");
    assert!(
        inst_url.contains("startDateTime="),
        "unbounded expansion: {inst_url}"
    );
}

/// A master that vanished between the listing and the expansion is not an
/// error: 404 skips it. Anything else still raises, so a real failure is not
/// swallowed into a silently short listing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_master_deleted_mid_walk_does_not_fail_the_run() {
    let master = json!({
        "id": "S1",
        "type": "seriesMaster",
        "recurrence": {
            "pattern": { "type": "daily", "interval": 1 },
            "range": { "type": "noEnd" },
        },
    });
    let run = call_adapter(
        list_input(),
        vec![
            page(vec![master]),
            json!({ "status": 404, "headers": {}, "body": { "error": { "message": "gone" } } }),
        ],
    )
    .await;
    let items = run.output.expect("list output")["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["external_id"], json!("S1"));
}

// ---------------------------------------------------------------------------
// Cancelled occurrences and the UNTIL cutoff.
//
// These four cases lived in `index.test.mjs`, a hand-run `node --test` script
// that loaded the adapter as a bare script through `new Function`. Splitting the
// adapter into ES modules ended that: the entry is now a module graph, and a
// harness that concatenates one file cannot represent it. They are ported here
// rather than dropped — this suite executes the REAL files through the REAL
// module loader, so it covers strictly more than the script did, and it runs in
// CI instead of by hand.
//
// Both defects they pin are invisible in every log: a cancelled occurrence that
// produced no item left the expander regenerating a called-off meeting forever,
// and an `UNTIL` rendered at the end of the endDate in UTC fell BEFORE the final
// occurrence of every series west of UTC, so that occurrence silently vanished.

const MASTER_ID: &str = "AAMkMASTER";
const OCC_ID: &str = "OID.AAMkMASTER.2026-09-15";

/// A weekly series master ending 2026-12-31, at `local` in `zone`.
fn weekly_master(local: &str, zone: &str, all_day: bool) -> Value {
    json!({
        "id": MASTER_ID,
        "type": "seriesMaster",
        "subject": "Standup",
        "isAllDay": all_day,
        "start": { "dateTime": local, "timeZone": zone },
        "end": { "dateTime": local, "timeZone": zone },
        "recurrence": {
            "pattern": { "type": "weekly", "interval": 1, "daysOfWeek": ["tuesday"] },
            "range": { "type": "endDate", "startDate": "2026-01-06", "endDate": "2026-12-31" },
        },
    })
}

fn body(v: Value) -> Value {
    json!({ "status": 200, "headers": {}, "body": v })
}

/// The RRULE of a single-master listing, with the `/instances` expansion and the
/// cancelled-occurrence probe both answered empty.
async fn master_rrule(master: Value) -> String {
    let run = call_adapter(
        list_input(),
        vec![
            page(vec![master]),
            page(vec![]),
            body(json!({ "id": MASTER_ID, "cancelledOccurrences": [] })),
        ],
    )
    .await;
    run.output.expect("list output")["items"][0]["metadata"]["recurrence"][0]
        .as_str()
        .expect("an RRULE line")
        .to_string()
}

/// A cancelled occurrence must MATERIALIZE as an exception item.
///
/// With no item there is no exception node, and the expander — which suppresses
/// a slot only for `recurrence_type: exception` carrying an `original_start` —
/// keeps projecting the meeting that was called off on every rebuild, forever
/// and silently. Typing it as an `occurrence` instead would be no better: it
/// would then look like one of the expander's own projection nodes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_occurrence_becomes_an_exception_item_not_silence() {
    let run = call_adapter(
        list_input(),
        vec![
            page(vec![weekly_master("2026-01-06T09:00:00.0000000", "W. Europe Standard Time", false)]),
            page(vec![]),
            body(json!({ "id": MASTER_ID, "cancelledOccurrences": [OCC_ID] })),
            body(json!({
                "id": OCC_ID,
                "type": "occurrence",
                "seriesMasterId": MASTER_ID,
                "isCancelled": true,
                "originalStart": "2026-09-15T07:00:00Z",
                "subject": "Standup",
                "start": { "dateTime": "2026-09-15T09:00:00.0000000", "timeZone": "W. Europe Standard Time" },
                "end": { "dateTime": "2026-09-15T09:15:00.0000000", "timeZone": "W. Europe Standard Time" },
            })),
        ],
    )
    .await;

    let items = run.output.expect("list output")["items"]
        .as_array()
        .expect("items")
        .clone();
    let cancelled = items
        .iter()
        .find(|i| i["external_id"] == json!(OCC_ID))
        .unwrap_or_else(|| panic!("no item for the cancelled occurrence: {items:#?}"));
    let meta = &cancelled["metadata"];
    assert_eq!(meta["event_type"], json!("exception"));
    assert_eq!(meta["series_master_id"], json!(MASTER_ID));
    assert_eq!(meta["original_start"], json!("2026-09-15T07:00:00Z"));
    assert_eq!(meta["is_cancelled"], json!(true));
}

/// A Graph endpoint that will not answer the cancelled-occurrence probe costs
/// nothing: the walk still lists the series. The probe is an enrichment, and a
/// tenant where it 404s must not lose its calendar.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_cancelled_occurrence_probe_still_lists_the_series() {
    let run = call_adapter(
        list_input(),
        vec![
            page(vec![weekly_master("2026-01-06T09:00:00.0000000", "W. Europe Standard Time", false)]),
            json!({ "status": 404, "headers": {}, "body": {} }),
            json!({ "status": 404, "headers": {}, "body": {} }),
        ],
    )
    .await;
    let items = run.output.expect("list output")["items"].clone();
    assert_eq!(items.as_array().map(|a| a.len()), Some(1), "{items:#?}");
}

/// 19:00 America/New_York on 2026-12-31 is 2027-01-01T00:00:00Z. The old
/// rendering — end of the endDate in UTC — is BEFORE that, so `rrule` dropped
/// the last instance of every US-evening series with no error anywhere. The
/// cutoff must clear it, and must NOT reach so far that a DAILY series gains an
/// occurrence: the next slot is at least a day later, and the 12-hour pad stays
/// inside that gap for every zone from UTC-12 through UTC+11.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn until_covers_the_final_occurrence_for_a_series_west_of_utc() {
    let rrule = master_rrule(weekly_master(
        "2026-01-06T19:00:00.0000000",
        "Eastern Standard Time",
        false,
    ))
    .await;
    let until = rrule
        .split("UNTIL=")
        .nth(1)
        .expect("an UNTIL")
        .trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string();
    assert!(
        until.as_str() > "20261231T235959Z",
        "UNTIL={until} still precedes the final occurrence's instant"
    );
    assert!(
        until.as_str() < "20270101T190000Z",
        "UNTIL={until} reaches into the next day's slot"
    );
}

/// An all-day series carries a bare date, so there is no local time of day to
/// pad from — and end of day is then the whole of the truth there is. The same
/// 12-hour cover applies from 00:00:00, so the cutoff must still exist.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_all_day_series_still_gets_an_until() {
    let rrule = master_rrule(weekly_master("2026-01-06T00:00:00.0000000", "UTC", true)).await;
    assert!(
        rrule.contains("UNTIL=20261231T120000Z"),
        "got {rrule}"
    );
}
