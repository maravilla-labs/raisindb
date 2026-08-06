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
