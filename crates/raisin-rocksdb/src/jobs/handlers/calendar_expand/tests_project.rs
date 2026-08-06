//! Projection shape: deterministic paths, and what an occurrence node carries.

use chrono::{TimeZone, Utc};

use super::master::{parse_exception, parse_master};
use super::project::{occurrence_path, occurrence_properties};
use super::rule::expand;
use super::tests_fixtures::{exception_node, s, weekly_master};

#[test]
fn occurrence_paths_are_deterministic_and_sort_chronologically() {
    let a = occurrence_path("m1", Utc.with_ymd_and_hms(2026, 3, 31, 7, 0, 0).unwrap());
    let b = occurrence_path("m1", Utc.with_ymd_and_hms(2026, 4, 7, 7, 0, 0).unwrap());
    assert_eq!(a, "/_occurrences/m1/2026/03/20260331T070000Z");
    assert_eq!(b, "/_occurrences/m1/2026/04/20260407T070000Z");
    assert!(a < b, "occurrence names must sort chronologically");
    assert_eq!(
        a,
        occurrence_path("m1", Utc.with_ymd_and_hms(2026, 3, 31, 7, 0, 0).unwrap())
    );
}

/// An occurrence must never carry its master's rule: it would expand again on
/// the next rebuild, and every generated occurrence would become a master.
#[test]
fn an_occurrence_never_carries_the_recurrence_rule_or_reserved_props() {
    let master = parse_master(&weekly_master("m1", Some("ext-1"))).unwrap();
    let out = expand(
        &master,
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap(),
        10,
    )
    .unwrap();
    let props = occurrence_properties(&master, &out.occurrences[0]);
    assert!(!props.contains_key("recurrence"));
    assert!(props.keys().all(|k| !k.starts_with("__")));
    assert_eq!(props.get("recurrence_type"), Some(&s("occurrence")));
    assert_eq!(props.get("title"), Some(&s("Standup")));
    assert_eq!(props.get("series_master_external_id"), Some(&s("ext-1")));
    assert_eq!(props.get("original_start_utc"), props.get("start_utc"));
}

#[test]
fn an_exception_parses_only_with_both_halves_of_its_identity() {
    let node = exception_node("e1", "ext-1", "2026-03-17T08:00:00Z", "confirmed");
    let slot = parse_exception(&node).unwrap();
    assert_eq!(slot.series_master_external_id, "ext-1");
    assert_eq!(slot.original_start_utc, "2026-03-17T08:00:00Z");

    let mut half = node.clone();
    half.properties.remove("original_start_utc");
    assert!(parse_exception(&half).is_none());
}
