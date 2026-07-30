//! Direct tests of the compaction filter's decision logic.
//!
//! These drive [`SpatialPruneFilter::decide`] with SYNTHETIC keys rather than
//! going through RocksDB, because what has to be pinned is the state machine —
//! which entry is the "first seen" for a node, when the seen-set resets, and the
//! two directions in which a wrong answer is catastrophic (dropping a live entry
//! resurrects nothing but LOSES a row; dropping a tombstone RESURRECTS deleted
//! geometry). An end-to-end compaction test cannot isolate those.

use super::compaction::SpatialPruneFilter;
use super::compaction_config::SpatialCompactionConfig;
use crate::indexing::SPATIAL_TOMBSTONE;
use crate::keys::spatial_index_key_versioned;
use raisin_hlc::HLC;
use rocksdb::compaction_filter::Decision;

const NOW_MS: u64 = 1_800_000_000_000;

fn key(property: &str, geohash: &str, ts_ms: u64, node_id: &str) -> Vec<u8> {
    spatial_index_key_versioned(
        "t",
        "r",
        "main",
        "ws",
        property,
        geohash,
        &HLC::new(ts_ms, 0),
        node_id,
    )
}

fn live() -> &'static [u8] {
    b"live-entry-value"
}

fn removed(d: Decision) -> bool {
    matches!(d, Decision::Remove)
}

/// `newest_only` + a fixed clock: the sharpest configuration, so the state
/// machine is what is under test rather than the retention arithmetic.
fn filter(full_compaction: bool) -> SpatialPruneFilter {
    SpatialPruneFilter::with_clock(
        SpatialCompactionConfig::newest_only(),
        full_compaction,
        NOW_MS,
    )
}

/// Keys arrive newest-revision-first inside a cell prefix, so descending
/// timestamps here mirror what RocksDB actually presents.
#[test]
fn keeps_the_newest_revision_and_drops_every_superseded_one() {
    let mut f = filter(false);
    let ts = [500u64, 400, 300, 200, 100];
    let decisions: Vec<bool> = ts
        .iter()
        .map(|&t| removed(f.decide(&key("position", "u0nc", t, "veh1"), live())))
        .collect();

    assert_eq!(
        decisions,
        vec![false, true, true, true, true],
        "the first (newest) entry is kept, every older one is superseded"
    );
    assert_eq!(f.stats().removed_superseded, 4);
}

/// The counter-intuitive property from OPEN-ITEMS §2.99: within a cell prefix
/// the key orders by REVISION first and `node_id` only as a tiebreak, so two
/// tracked objects' revisions INTERLEAVE. Each must keep its own newest.
#[test]
fn interleaved_nodes_each_keep_their_own_newest() {
    let mut f = filter(false);
    // Real interleaving: a@500, b@450, a@400, b@350, a@300 ...
    let seq = [
        ("a", 500u64),
        ("b", 450),
        ("a", 400),
        ("b", 350),
        ("a", 300),
        ("b", 250),
    ];
    let decisions: Vec<bool> = seq
        .iter()
        .map(|&(n, t)| removed(f.decide(&key("position", "u0nc", t, n), live())))
        .collect();

    assert_eq!(
        decisions,
        vec![false, false, true, true, true, true],
        "a@500 and b@450 are each their node's newest and survive; the rest do not"
    );
}

/// State is per cell prefix. If it did not reset, the second cell's newest entry
/// would be treated as superseded and dropped — a live entry lost.
#[test]
fn state_resets_when_the_cell_prefix_changes() {
    let mut f = filter(false);
    assert!(!removed(
        f.decide(&key("position", "u0nc", 500, "veh1"), live())
    ));
    assert!(removed(
        f.decide(&key("position", "u0nc", 400, "veh1"), live())
    ));

    // Different geohash, same node: this is that cell's newest.
    assert!(
        !removed(f.decide(&key("position", "u0nd", 300, "veh1"), live())),
        "a new cell prefix must reset the per-node state"
    );
    // Different property, same geohash: also a distinct prefix.
    assert!(!removed(
        f.decide(&key("home", "u0nd", 200, "veh1"), live())
    ));
    // Different branch: distinct prefix again, or branches would prune each other.
    let other_branch = spatial_index_key_versioned(
        "t",
        "r",
        "dev",
        "ws",
        "home",
        "u0nd",
        &HLC::new(150, 0),
        "veh1",
    );
    assert!(!removed(f.decide(&other_branch, live())));
}

/// A tombstone shadows older live entries. Dropping it outside a full compaction
/// could unshadow one that lives in a file this run cannot see — resurrecting
/// deleted geometry.
#[test]
fn tombstone_is_kept_when_the_compaction_is_not_full() {
    let mut f = filter(false);
    assert!(
        !removed(f.decide(&key("position", "u0nc", 500, "veh1"), SPATIAL_TOMBSTONE)),
        "a tombstone must survive a partial compaction"
    );
    // The older live entry it shadows is still superseded and may go.
    assert!(removed(
        f.decide(&key("position", "u0nc", 400, "veh1"), live())
    ));
    assert_eq!(f.stats().removed_tombstones, 0);
}

/// In a FULL compaction every older entry for the node is in this same run and
/// is removed by this same pass, so nothing can be unshadowed.
#[test]
fn tombstone_and_everything_under_it_go_in_a_full_compaction() {
    let mut f = filter(true);
    assert!(removed(f.decide(
        &key("position", "u0nc", 500, "veh1"),
        SPATIAL_TOMBSTONE
    )));
    assert!(removed(
        f.decide(&key("position", "u0nc", 400, "veh1"), live())
    ));
    assert!(removed(
        f.decide(&key("position", "u0nc", 300, "veh1"), live())
    ));
    assert_eq!(f.stats().removed_tombstones, 1);
    assert_eq!(f.stats().removed_superseded, 2);
}

/// A tombstone that is NOT a node's newest entry in this run is itself
/// superseded by a newer live entry (the node moved back into this cell), so it
/// is ordinary superseded data.
#[test]
fn superseded_tombstone_is_dropped_like_any_other_older_entry() {
    let mut f = filter(false);
    assert!(!removed(
        f.decide(&key("position", "u0nc", 500, "veh1"), live())
    ));
    assert!(removed(f.decide(
        &key("position", "u0nc", 400, "veh1"),
        SPATIAL_TOMBSTONE
    )));
}

#[test]
fn malformed_keys_are_always_kept() {
    let mut f = filter(true);
    for bad in [
        b"".as_slice(),
        b"not-a-spatial-key".as_slice(),
        b"t\0r\0main\0ws\0notgeo\0prop\0u0nc\0".as_slice(),
        // Correct shape but truncated inside the fixed-width revision field.
        b"t\0r\0main\0ws\0geo\0prop\0u0nc\0short".as_slice(),
    ] {
        assert!(
            !removed(f.decide(bad, live())),
            "an unparseable key must never be removed: {:?}",
            bad
        );
    }
    assert_eq!(f.stats().unparseable, 4);
    assert_eq!(f.stats().removed_superseded, 0);
}

#[test]
fn disabled_filter_removes_nothing() {
    let mut f = SpatialPruneFilter::with_clock(SpatialCompactionConfig::disabled(), true, NOW_MS);
    for t in [500u64, 400, 300] {
        assert!(!removed(
            f.decide(&key("position", "u0nc", t, "veh1"), live())
        ));
    }
    assert!(!removed(f.decide(
        &key("position", "u0nc", 200, "veh1"),
        SPATIAL_TOMBSTONE
    )));
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// The count budget: `keep_revisions` entries per node per cell, newest first.
#[test]
fn keep_revisions_bounds_how_much_history_survives() {
    let config = SpatialCompactionConfig {
        keep_revisions: 3,
        retention_secs: 86_400,
        ..SpatialCompactionConfig::default()
    };
    let mut f = SpatialPruneFilter::with_clock(config, false, NOW_MS);

    // Five entries, all recent enough to be inside the time horizon.
    let decisions: Vec<bool> = (0..5)
        .map(|i| {
            let ts = NOW_MS - i * 1_000;
            removed(f.decide(&key("position", "u0nc", ts, "veh1"), live()))
        })
        .collect();
    assert_eq!(decisions, vec![false, false, false, true, true]);
}

/// The time horizon: an entry inside the count budget still goes if it has aged
/// out. Both bounds are needed — the horizon alone does not bound a hot cell.
#[test]
fn retention_horizon_drops_entries_the_count_budget_would_have_kept() {
    let config = SpatialCompactionConfig {
        keep_revisions: 100,
        retention_secs: 60,
        ..SpatialCompactionConfig::default()
    };
    let mut f = SpatialPruneFilter::with_clock(config, false, NOW_MS);

    assert!(!removed(f.decide(
        &key("position", "u0nc", NOW_MS - 10_000, "veh1"),
        live()
    )));
    // 30 s old: inside the 60 s horizon.
    assert!(!removed(f.decide(
        &key("position", "u0nc", NOW_MS - 30_000, "veh1"),
        live()
    )));
    // 90 s old: outside it.
    assert!(removed(f.decide(
        &key("position", "u0nc", NOW_MS - 90_000, "veh1"),
        live()
    )));
}

/// The newest entry is NEVER dropped, however old it is. A static place written
/// once years ago must keep matching queries at HEAD.
#[test]
fn the_newest_entry_survives_regardless_of_age() {
    let config = SpatialCompactionConfig {
        keep_revisions: 1,
        retention_secs: 1,
        ..SpatialCompactionConfig::default()
    };
    let mut f = SpatialPruneFilter::with_clock(config, false, NOW_MS);
    assert!(!removed(
        f.decide(&key("location", "u0nc", 1, "place"), live())
    ));
}

/// A tombstone still inside the retention window survives even in a full
/// compaction: dropping it would remove the evidence a historical read needs
/// that the node was deleted.
#[test]
fn tombstone_inside_the_retention_window_survives_a_full_compaction() {
    let config = SpatialCompactionConfig {
        retention_secs: 3_600,
        ..SpatialCompactionConfig::default()
    };
    let mut f = SpatialPruneFilter::with_clock(config, true, NOW_MS);
    assert!(!removed(f.decide(
        &key("position", "u0nc", NOW_MS - 1_000, "veh1"),
        SPATIAL_TOMBSTONE
    )));
    assert_eq!(f.stats().removed_tombstones, 0);
}

/// The memory valve: once a prefix has more distinct nodes than the cap, the
/// filter stops pruning it entirely rather than growing its map without bound.
#[test]
fn a_prefix_over_the_node_budget_is_left_alone() {
    let config = SpatialCompactionConfig {
        max_tracked_nodes_per_cell: 2,
        ..SpatialCompactionConfig::newest_only()
    };
    let mut f = SpatialPruneFilter::with_clock(config, false, NOW_MS);
    assert!(!removed(
        f.decide(&key("position", "u0nc", 500, "a"), live())
    ));
    assert!(!removed(
        f.decide(&key("position", "u0nc", 490, "b"), live())
    ));
    // Third distinct node trips the valve; nothing more in this prefix is pruned.
    assert!(!removed(
        f.decide(&key("position", "u0nc", 480, "c"), live())
    ));
    assert!(!removed(
        f.decide(&key("position", "u0nc", 400, "a"), live())
    ));
}

/// Revisions stamped in the future (cluster clock skew) must be kept, not
/// treated as infinitely old.
#[test]
fn future_revisions_are_treated_as_in_window() {
    let config = SpatialCompactionConfig {
        keep_revisions: 4,
        retention_secs: 60,
        ..SpatialCompactionConfig::default()
    };
    let mut f = SpatialPruneFilter::with_clock(config, false, NOW_MS);
    assert!(!removed(f.decide(
        &key("position", "u0nc", NOW_MS + 600_000, "veh1"),
        live()
    )));
    assert!(!removed(f.decide(
        &key("position", "u0nc", NOW_MS + 300_000, "veh1"),
        live()
    )));
}
