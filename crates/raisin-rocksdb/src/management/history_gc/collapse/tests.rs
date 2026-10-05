//! The run detector over hand-built PROPERTY_INDEX and ORDERED_CHILDREN keys,
//! fed in CF order.

use super::detector::{Detector, EntryState};
use super::{target, CfCollapseCounts};
use crate::{cf, keys};
use raisin_hlc::HLC;

fn prop(value: &str, rev: u64, node: &str) -> Vec<u8> {
    keys::property_index_key_versioned(
        "t",
        "r",
        "main",
        "ws",
        "title",
        value,
        &HLC::new(rev, 0),
        node,
        false,
    )
}

/// Feed `(key, value)` in sorted key order; return the deleted keys.
fn run(entries: Vec<(Vec<u8>, &[u8])>, cf_name: &str, watermark: u64) -> Vec<Vec<u8>> {
    let mut entries = entries;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut detector = Detector::new(target(cf_name), HLC::new(watermark, 0));
    let mut counts = CfCollapseCounts::default();
    let mut doomed = Vec::new();
    for (key, value) in &entries {
        if let Some(d) = detector.visit(key, value, &mut counts).unwrap() {
            doomed.push(d.key);
        }
    }
    doomed
}

#[test]
fn a_run_keeps_only_its_oldest_version() {
    let doomed = run(
        vec![
            (prop("a", 1, "n1"), b"n1"),
            (prop("a", 2, "n1"), b"n1"),
            (prop("a", 3, "n1"), b"n1"),
        ],
        cf::PROPERTY_INDEX,
        100,
    );
    let mut expected = vec![prop("a", 3, "n1"), prop("a", 2, "n1")];
    expected.sort();
    let mut doomed = doomed;
    doomed.sort();
    assert_eq!(doomed, expected);
}

#[test]
fn a_state_change_ends_the_run() {
    // live, T, live: every version is a change.
    let doomed = run(
        vec![
            (prop("a", 1, "n1"), b"n1"),
            (prop("a", 2, "n1"), keys::TOMBSTONE_VALUE),
            (prop("a", 3, "n1"), b"n1"),
        ],
        cf::PROPERTY_INDEX,
        100,
    );
    assert!(doomed.is_empty());
    // T after T is a run too.
    let doomed = run(
        vec![
            (prop("a", 1, "n1"), b"n1"),
            (prop("a", 2, "n1"), keys::TOMBSTONE_VALUE),
            (prop("a", 3, "n1"), keys::TOMBSTONE_VALUE),
        ],
        cf::PROPERTY_INDEX,
        100,
    );
    assert_eq!(doomed, vec![prop("a", 3, "n1")]);
}

#[test]
fn different_bytes_are_different_states() {
    let doomed = run(
        vec![(prop("a", 1, "n1"), b"x"), (prop("a", 2, "n1"), b"y")],
        cf::PROPERTY_INDEX,
        100,
    );
    assert!(doomed.is_empty());
}

#[test]
fn nodes_sharing_a_value_are_separate_groups() {
    // n1 and n2 interleave by revision inside one chunk.
    let doomed = run(
        vec![
            (prop("a", 1, "n1"), b"n1"),
            (prop("a", 2, "n2"), b"n2"),
            (prop("a", 3, "n1"), b"n1"),
            (prop("a", 4, "n2"), keys::TOMBSTONE_VALUE),
        ],
        cf::PROPERTY_INDEX,
        100,
    );
    assert_eq!(doomed, vec![prop("a", 3, "n1")]);
}

#[test]
fn only_versions_strictly_below_the_watermark_go() {
    let entries = || {
        vec![
            (prop("a", 1, "n1"), b"n1".as_slice()),
            (prop("a", 2, "n1"), b"n1".as_slice()),
            (prop("a", 3, "n1"), b"n1".as_slice()),
        ]
    };
    assert_eq!(
        run(entries(), cf::PROPERTY_INDEX, 3),
        vec![prop("a", 2, "n1")]
    );
    assert!(run(entries(), cf::PROPERTY_INDEX, 2).is_empty());
}

#[test]
fn ordered_children_group_by_label_and_child() {
    let key = |label: &str, rev: u64, child: &str| {
        keys::ordered_child_key_versioned(
            "t",
            "r",
            "main",
            "ws",
            "p",
            label,
            &HLC::new(rev, 0),
            child,
        )
    };
    let doomed = run(
        vec![
            (key("a0", 1, "c"), b"c".as_slice()),
            (key("a0", 2, "c"), b"c".as_slice()),
            // Reordered: the old label ends, the new one starts.
            (key("a0", 3, "c"), keys::TOMBSTONE_VALUE),
            (key("b0", 3, "c"), b"c".as_slice()),
            (key("b0", 4, "c"), b"c".as_slice()),
        ],
        cf::ORDERED_CHILDREN,
        100,
    );
    let mut doomed = doomed;
    doomed.sort();
    let mut expected = vec![key("a0", 2, "c"), key("b0", 4, "c")];
    expected.sort();
    assert_eq!(doomed, expected);
}

#[test]
fn the_state_compares_tombstone_spellings_alike() {
    assert_eq!(EntryState::of(b"T"), EntryState::of(b"\x00"));
    assert_ne!(EntryState::of(b"T"), EntryState::of(b"n1"));
}

fn ordered(label: &str, rev: u64, child: &str) -> Vec<u8> {
    keys::ordered_child_key_versioned("t", "r", "main", "ws", "p", label, &HLC::new(rev, 0), child)
}

#[test]
fn collapse_keeps_editorial_order_of_children_sharing_a_label() {
    // a and b share label L (a verbatim merge copy). The readers order them
    // by key position, so a@10 vs b@5 lists a first; deleting a@10 (a twin
    // of a@2) would list b first at every revision >= 5.
    let doomed = run(
        vec![
            (ordered("L", 2, "a"), b"a".as_slice()),
            (ordered("L", 5, "b"), b"b".as_slice()),
            (ordered("L", 10, "a"), b"a".as_slice()),
        ],
        cf::ORDERED_CHILDREN,
        100,
    );
    assert!(doomed.is_empty(), "an interleaved twin went");

    // A same-revision tie is an interleaving too: b@10 sorts after a@10.
    let doomed = run(
        vec![
            (ordered("L", 2, "a"), b"a".as_slice()),
            (ordered("L", 10, "a"), b"a".as_slice()),
            (ordered("L", 10, "b"), b"b".as_slice()),
        ],
        cf::ORDERED_CHILDREN,
        100,
    );
    assert!(doomed.is_empty(), "a twin across a tie went");

    // Adjacent twins still go, beside another child.
    let doomed = run(
        vec![
            (ordered("L", 2, "a"), b"a".as_slice()),
            (ordered("L", 3, "a"), b"a".as_slice()),
            (ordered("L", 5, "b"), b"b".as_slice()),
        ],
        cf::ORDERED_CHILDREN,
        100,
    );
    assert_eq!(doomed, vec![ordered("L", 3, "a")]);

    // PROPERTY_INDEX has no order to keep: interleaved twins go.
    let doomed = run(
        vec![
            (prop("a", 2, "n1"), b"n1".as_slice()),
            (prop("a", 5, "n2"), b"n2".as_slice()),
            (prop("a", 10, "n1"), b"n1".as_slice()),
        ],
        cf::PROPERTY_INDEX,
        100,
    );
    assert_eq!(doomed, vec![prop("a", 10, "n1")]);
}

#[test]
fn hot_chunk_memory_is_capped_and_forgetting_only_keeps() {
    // n1@4, n3@3, n2@2, n1@1 in key order: with room for two groups, n1 is
    // the least recently seen when n2 arrives, and is forgotten (and n3 when
    // n1 comes back as a new group).
    let mut entries = vec![
        (prop("a", 1, "n1"), b"n1".as_slice()),
        (prop("a", 2, "n2"), b"n2".as_slice()),
        (prop("a", 3, "n3"), b"n3".as_slice()),
        (prop("a", 4, "n1"), b"n1".as_slice()),
    ];
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let target = target(cf::PROPERTY_INDEX);
    let mut counts = CfCollapseCounts::default();
    let mut capped = Detector::new(target, HLC::new(100, 0)).with_max_groups(2);
    let mut doomed = Vec::new();
    for (key, value) in &entries {
        if let Some(d) = capped.visit(key, value, &mut counts).unwrap() {
            doomed.push(d.key);
        }
    }
    assert!(doomed.is_empty());
    assert_eq!(counts.forgotten_groups, 2);
    // Uncapped, n1@4 is redundant.
    assert_eq!(
        run(entries, cf::PROPERTY_INDEX, 100),
        vec![prop("a", 4, "n1")]
    );
}

#[test]
fn a_version_remembered_from_an_earlier_slice_is_re_read() {
    let target = target(cf::PROPERTY_INDEX);
    let mut counts = CfCollapseCounts::default();
    let mut detector = Detector::new(target, HLC::new(100, 0));
    let (newer, older) = (prop("a", 2, "n1"), prop("a", 1, "n1"));
    detector.begin_slice();
    assert!(detector
        .visit(&newer, b"n1", &mut counts)
        .unwrap()
        .is_none());
    detector.begin_slice();
    let doomed = detector.visit(&older, b"n1", &mut counts).unwrap().unwrap();
    assert_eq!(doomed.key, newer);
    assert!(doomed.stale, "read by the previous slice");
    assert_eq!(doomed.state, EntryState::Live(b"n1".to_vec()));
}
