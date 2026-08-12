//! The pure expansion, including the DST property test that is the point of the
//! whole module.

use chrono::{TimeZone, Utc};
use raisin_models::nodes::properties::PropertyValue;

use super::format_utc;
use super::master::parse_master;
use super::rule::{build_ical, expand, ExpandError};
use super::tests_fixtures::{s, weekly_master};

// ------------------------------------------------------- pure expansion

/// THE test. "Every Tuesday 09:00 Europe/Zurich" is 08:00Z under CET and 07:00Z
/// under CEST. An implementation that adds a fixed offset to a UTC anchor —
/// which is what every naive expander does — produces 08:00Z on both sides and
/// passes every non-DST test there is.
#[test]
fn weekly_local_time_survives_the_spring_forward() {
    let master = parse_master(&weekly_master("m1", Some("ext-1"))).unwrap();
    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap(),
        50,
    )
    .unwrap();
    assert!(!out.truncated);

    let starts: Vec<String> = out
        .occurrences
        .iter()
        .map(|o| format_utc(o.start_utc))
        .collect();
    assert_eq!(
        starts,
        vec![
            // CET (UTC+1) — before the 2026-03-29 transition.
            "2026-03-10T08:00:00Z",
            "2026-03-17T08:00:00Z",
            "2026-03-24T08:00:00Z",
            // CEST (UTC+2) — after it.
            "2026-03-31T07:00:00Z",
            "2026-04-07T07:00:00Z",
            "2026-04-14T07:00:00Z",
        ]
    );
    // The wall clock a user agreed to is the thing that stayed constant.
    for occ in &out.occurrences {
        assert!(
            occ.start_local.ends_with("T09:00:00"),
            "local wall clock drifted: {}",
            occ.start_local
        );
    }
    // Duration is carried, not recomputed: 30 minutes on both sides.
    for occ in &out.occurrences {
        assert_eq!(occ.end_utc - occ.start_utc, chrono::Duration::minutes(30));
    }
}

/// A daily 02:30 series crosses a local time that DOES NOT EXIST on
/// 2026-03-29, when Europe/Zurich jumps 02:00 → 03:00.
///
/// Defined behaviour: the instance is SHIFTED FORWARD by the gap, never
/// dropped and never silently duplicated. Dropping it would make a daily series
/// mysteriously skip one day a year; shifting forward keeps the elapsed
/// spacing (01:30Z on every day of the run) and matches what both providers do.
#[test]
fn spring_forward_gap_shifts_the_instance_forward() {
    let mut node = weekly_master("m-gap", None);
    node.properties.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![s("RRULE:FREQ=DAILY")]),
    );
    // 02:30 Europe/Zurich on 2026-03-27 is 01:30Z.
    node.properties
        .insert("start_utc".into(), s("2026-03-27T01:30:00Z"));
    node.properties
        .insert("end_utc".into(), s("2026-03-27T02:30:00Z"));
    let master = parse_master(&node).unwrap();

    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 27, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 3, 31, 0, 0, 0).unwrap(),
        20,
    )
    .unwrap();

    let pairs: Vec<(String, String)> = out
        .occurrences
        .iter()
        .map(|o| (format_utc(o.start_utc), o.start_local.clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("2026-03-27T01:30:00Z".into(), "2026-03-27T02:30:00".into()),
            ("2026-03-28T01:30:00Z".into(), "2026-03-28T02:30:00".into()),
            // The gap day: 02:30 does not exist, so it lands at 03:30 CEST —
            // the same instant the un-shifted spacing implies.
            ("2026-03-29T01:30:00Z".into(), "2026-03-29T03:30:00".into()),
            ("2026-03-30T00:30:00Z".into(), "2026-03-30T02:30:00".into()),
        ]
    );
}

/// A rule with no COUNT and no UNTIL is an infinite set. Expansion must be
/// bounded by BOTH the window and the cap, and a truncated result must say so.
#[test]
fn infinite_rule_is_bounded_and_reports_truncation() {
    let mut node = weekly_master("m-inf", None);
    node.properties.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![s("RRULE:FREQ=MINUTELY")]),
    );
    let master = parse_master(&node).unwrap();
    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 10, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2027, 3, 10, 0, 0, 0).unwrap(),
        25,
    )
    .unwrap();
    assert_eq!(out.occurrences.len(), 25);
    assert!(out.truncated, "an infinite rule must report truncation");
}

#[test]
fn exdate_removes_its_instance() {
    let mut node = weekly_master("m-ex", None);
    node.properties.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![
            s("RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=4"),
            s("EXDATE;TZID=Europe/Zurich:20260317T090000"),
        ]),
    );
    let master = parse_master(&node).unwrap();
    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
        50,
    )
    .unwrap();
    let starts: Vec<String> = out
        .occurrences
        .iter()
        .map(|o| format_utc(o.start_utc))
        .collect();
    assert_eq!(
        starts,
        vec![
            "2026-03-10T08:00:00Z",
            "2026-03-24T08:00:00Z",
            "2026-03-31T07:00:00Z",
        ]
    );
}

#[test]
fn a_bare_rule_body_is_promoted_and_an_embedded_dtstart_is_dropped() {
    let mut node = weekly_master("m-bare", None);
    node.properties.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![
            // A provider-supplied DTSTART must never win over the master's own
            // start; honouring it lets the two disagree with no error.
            s("DTSTART;TZID=Europe/Zurich:20200101T000000"),
            s("FREQ=WEEKLY;BYDAY=TU;COUNT=2"),
        ]),
    );
    let master = parse_master(&node).unwrap();
    let ical = build_ical(&master).unwrap();
    assert!(ical.starts_with("DTSTART;TZID=Europe/Zurich:20260310T090000"));
    assert!(ical.contains("RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=2"));
    assert_eq!(ical.matches("DTSTART").count(), 1);

    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
        10,
    )
    .unwrap();
    assert_eq!(out.occurrences.len(), 2);
    assert_eq!(
        format_utc(out.occurrences[0].start_utc),
        "2026-03-10T08:00:00Z"
    );
}

#[test]
fn a_bad_timezone_and_a_bad_rule_are_reported_not_guessed() {
    let mut node = weekly_master("m-tz", None);
    node.properties
        .insert("timezone".into(), s("Mars/Olympus_Mons"));
    let master = parse_master(&node).unwrap();
    assert!(matches!(
        build_ical(&master),
        Err(ExpandError::BadTimezone(_))
    ));

    let mut node = weekly_master("m-rule", None);
    node.properties.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![s("RRULE:FREQ=FORTNIGHTLY")]),
    );
    let master = parse_master(&node).unwrap();
    let err = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
        10,
    )
    .unwrap_err();
    assert!(matches!(err, ExpandError::BadRule(_)), "got {err:?}");

    let mut node = weekly_master("m-norule", None);
    node.properties
        .insert("recurrence".into(), PropertyValue::Array(vec![]));
    let master = parse_master(&node).unwrap();
    assert!(matches!(build_ical(&master), Err(ExpandError::NoRule)));
}

#[test]
fn a_master_without_a_timezone_expands_in_utc() {
    let mut node = weekly_master("m-noutz", None);
    node.properties.remove("timezone");
    let master = parse_master(&node).unwrap();
    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap(),
        10,
    )
    .unwrap();
    // No zone means no DST: 08:00Z on every side of the transition.
    for occ in &out.occurrences {
        assert!(format_utc(occ.start_utc).ends_with("T08:00:00Z"));
    }
}

#[test]
fn only_series_masters_parse_as_masters() {
    let mut node = weekly_master("m-single", None);
    node.properties
        .insert("recurrence_type".into(), s("single"));
    assert!(parse_master(&node).is_none());
    node.properties.remove("recurrence_type");
    assert!(parse_master(&node).is_none());
}

// ── deriving the instant a provider never reported ──────────────────────────
//
// Microsoft Graph reports a naive local datetime plus a separate timeZone and
// never a UTC instant, so the ms-graph mapper writes `start_utc: null` by
// design and defers the instant to "later, where a real tz database exists".
// Until that derivation existed, every Outlook series master failed with
// `master has no parsable start_utc` and produced no occurrences at all —
// silently, on a mount that reported a clean sync.

/// A master shaped the way ms-graph writes one: local wall clock + zone, no UTC.
fn local_only_master(id: &str, start_local: &str, end_local: &str) -> raisin_models::nodes::Node {
    let mut node = weekly_master(id, None);
    node.properties.remove("start_utc");
    node.properties.remove("end_utc");
    node.properties.insert("start_local".into(), s(start_local));
    node.properties.insert("end_local".into(), s(end_local));
    node
}

#[test]
fn a_master_with_only_local_time_still_expands() {
    let node = local_only_master("m-local", "2026-03-10T09:00:00", "2026-03-10T09:30:00");
    let master = parse_master(&node).unwrap();

    // 09:00 Europe/Zurich on 2026-03-10 is CET (UTC+1).
    assert_eq!(format_utc(master.start_utc.unwrap()), "2026-03-10T08:00:00Z");

    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap(),
        10,
    )
    .unwrap();
    assert!(!out.occurrences.is_empty(), "a local-only master must expand");

    // The end must be derived too, or every occurrence is zero-length while
    // still looking like a successful expansion.
    for occ in &out.occurrences {
        assert_eq!(
            occ.end_utc - occ.start_utc,
            chrono::Duration::minutes(30),
            "occurrence lost its duration"
        );
    }
}

#[test]
fn a_stored_utc_instant_still_wins_over_its_local_twin() {
    let mut node = weekly_master("m-both", None);
    // Deliberately disagreeing: the indexed UTC column is what everything else
    // orders by, so it must win rather than being recomputed from the local.
    node.properties
        .insert("start_local".into(), s("2026-03-10T17:00:00"));
    let master = parse_master(&node).unwrap();
    assert_eq!(format_utc(master.start_utc.unwrap()), "2026-03-10T08:00:00Z");
}

#[test]
fn the_ambiguous_dst_hour_resolves_to_the_earlier_instant() {
    // 2026-10-25 02:30 Europe/Zurich happens TWICE (CEST then CET).
    let node = local_only_master("m-ambiguous", "2026-10-25T02:30:00", "2026-10-25T03:00:00");
    let master = parse_master(&node).unwrap();
    // The first pass through 02:30 is 00:30Z (UTC+2); the second is 01:30Z.
    assert_eq!(format_utc(master.start_utc.unwrap()), "2026-10-25T00:30:00Z");
}

#[test]
fn the_nonexistent_dst_hour_steps_forward_instead_of_vanishing() {
    // 2026-03-29 02:30 Europe/Zurich never happens — the clock jumps 02:00→03:00.
    let node = local_only_master("m-gap", "2026-03-29T02:30:00", "2026-03-29T03:00:00");
    let master = parse_master(&node).unwrap();
    // Must resolve to the first instant that exists, not drop the master.
    assert_eq!(format_utc(master.start_utc.unwrap()), "2026-03-29T01:00:00Z");
}

#[test]
fn an_all_day_local_date_resolves_to_local_midnight() {
    let node = local_only_master("m-allday", "2026-03-10", "2026-03-11");
    let master = parse_master(&node).unwrap();
    // Midnight in Zurich, NOT midnight UTC — the whole reason the mapper
    // refuses to convert all-day values without a tz database.
    assert_eq!(format_utc(master.start_utc.unwrap()), "2026-03-09T23:00:00Z");
}

#[test]
fn no_zone_and_no_utc_is_still_reported_rather_than_guessed() {
    let mut node = local_only_master("m-nozone", "2026-03-10T09:00:00", "2026-03-10T09:30:00");
    node.properties.remove("timezone");
    let master = parse_master(&node).unwrap();
    // Guessing UTC here would silently move a meeting by the zone's offset.
    assert!(master.start_utc.is_none());
    assert_eq!(build_ical(&master).unwrap_err(), ExpandError::NoStart);
}
