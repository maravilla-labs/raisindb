//! The write guard: a derived occurrence is never writable.

use super::guard;
use super::tests_fixtures::{s, weekly_master};
use super::OCCURRENCE_ROOT;

#[test]
fn the_write_guard_refuses_projection_nodes_and_only_those() {
    assert!(guard::path_refusal("/_occurrences/m1/2026/03/20260331T070000Z").is_some());
    assert!(guard::path_refusal(OCCURRENCE_ROOT).is_some());
    // A sibling that merely shares the prefix is ordinary content.
    assert!(guard::path_refusal("/_occurrences_archive/thing").is_none());
    assert!(guard::path_refusal("/calendar/evt-1").is_none());

    let refusal = guard::path_refusal("/_occurrences/m1/2026/03/x").unwrap();
    assert!(refusal.contains("series master"), "{refusal}");
    assert!(refusal.contains("exception"), "{refusal}");

    // A projection node materialized somewhere unexpected is still refused, by
    // its own `recurrence_type`.
    let mut stray = weekly_master("stray", None);
    stray
        .properties
        .insert("recurrence_type".into(), s("occurrence"));
    stray.path = "/calendar/stray".into();
    assert!(guard::node_refusal(&stray).is_some());
    assert!(guard::node_refusal(&weekly_master("m", None)).is_none());
}
