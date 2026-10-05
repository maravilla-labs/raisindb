//! Regression tests for the Phase 8 review findings on the compound writer,
//! the delete tombstoner and the build pass.

use super::tests::{defs, doc, open, read, write, CTX};
use super::*;
use crate::indexing::Baseline;
use crate::keys::{self, TOMBSTONE_VALUE};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchScope;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// Store `node` (a legacy full blob, path embedded) as its version at `rev`;
/// `None` stores a delete tombstone.
fn store_version(db: &DB, node: Option<&Node>, rev: HLC) {
    let cf = crate::cf_handle(db, crate::cf::NODES).unwrap();
    let key = keys::node_key_versioned("t", "r", "main", "ws", "n1", &rev);
    let value = match node {
        Some(node) => rmp_serde::to_vec_named(node).unwrap(),
        None => TOMBSTONE_VALUE.to_vec(),
    };
    db.put_cf(cf, key, value).unwrap();
}

/// The `open` group's key for `n1` at `rev`.
fn open_key(rev: HLC) -> Vec<u8> {
    let group = compound_entries(defs().compound("t:Doc"), &CTX, &doc("open", 1))
        .into_iter()
        .next()
        .unwrap();
    entry_key(&group, &rev, "n1")
}

/// An ancestor move re-keys a node's entries WITHOUT a NODES version: here
/// its tombstone of the `open` tuple at 30. An ordinary write at 20 that keeps
/// `open` must put at 20 — landing on 30 overwrote the move's tombstone with
/// LIVE and resurrected the tuple at HEAD.
#[test]
fn ordinary_put_stays_below_a_newer_move_tombstone() {
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    let cf = crate::cf_handle(&db, crate::cf::COMPOUND_INDEX).unwrap();
    db.put_cf(cf, open_key(HLC::new(30, 0)), TOMBSTONE_VALUE)
        .unwrap();
    write(&db, Baseline::Full(Some(&v1)), &v1, HLC::new(20, 0));
    assert!(read(&db, "open", None).is_empty(), "the move's tombstone");
    assert_eq!(read(&db, "open", Some(HLC::new(25, 0))), vec!["n1"]);
}

/// An IN-PLACE write (a stored version at its own revision) still lands its
/// put on the newer entry, or that entry would mask it.
#[test]
fn in_place_put_still_lands_on_a_newer_entry() {
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    store_version(&db, Some(&v1), HLC::new(20, 0));
    let cf = crate::cf_handle(&db, crate::cf::COMPOUND_INDEX).unwrap();
    db.put_cf(cf, open_key(HLC::new(30, 0)), TOMBSTONE_VALUE)
        .unwrap();
    write(&db, Baseline::Full(Some(&v1)), &v1, HLC::new(20, 0));
    assert_eq!(read(&db, "open", None), vec!["n1"]);
}

/// A replicated delete arriving after a later version: the caller hands the
/// NEWEST version, but the delete ends the one in force at its revision.
#[test]
fn late_delete_ends_the_version_in_force_at_its_revision() {
    let (_dir, db) = open();
    let (v1, v3) = (doc("open", 1), doc("blocked", 1));
    store_version(&db, Some(&v1), HLC::new(10, 0));
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    store_version(&db, Some(&v3), HLC::new(30, 0));
    write(&db, Baseline::Full(Some(&v1)), &v3, HLC::new(30, 0));
    // Warm definitions, so the derived (not scanned) path runs.
    let key = cache::branch_key(&db, BranchScope::new("t", "r", "main"));
    let seen = cache::snapshot(&key);
    let doc_defs: TypeIndexDefs = TypeIndexDefs {
        compound: defs().compound("t:Doc").to_vec(),
        unique: vec![],
    };
    cache::store(&key, "t:Doc", Arc::new(doc_defs), seen);

    let mut batch = WriteBatch::default();
    tombstone_compound_for_delete(&mut batch, &db, &CTX, &v3, &HLC::new(20, 0)).unwrap();
    db.write(batch).unwrap();
    assert_eq!(read(&db, "open", Some(HLC::new(15, 0))), vec!["n1"]);
    assert!(
        read(&db, "open", Some(HLC::new(25, 0))).is_empty(),
        "deleted at 20: the version it deleted must not match after it"
    );
    assert_eq!(read(&db, "blocked", None), vec!["n1"]);
    assert!(read(&db, "blocked", Some(HLC::new(25, 0))).is_empty());
}

/// A version stored ABOVE the build's floor (a version stranded above HEAD,
/// or a write that committed while the build ran) is replayed at its own
/// revision, and the version at or below the floor is still the one a HEAD
/// read at the floor sees. The build used to index each node's newest
/// version only, so a HEAD-bounded read lost a node stranded above HEAD.
#[test]
fn build_indexes_the_floor_version_and_replays_versions_above_it() {
    let (_dir, db) = open();
    store_version(&db, Some(&doc("open", 1)), HLC::new(10, 0));
    store_version(&db, Some(&doc("done", 1)), HLC::new(30, 0));
    let wanted: build::Wanted = [("t:Doc".to_string(), defs().compound("t:Doc").to_vec())]
        .into_iter()
        .collect();
    let seen = build::write(&db, &CTX, &wanted, &HLC::new(20, 0)).unwrap();
    assert_eq!((seen.nodes, seen.unplaceable), (1, 0));
    assert_eq!(read(&db, "open", Some(HLC::new(20, 0))), vec!["n1"]);
    assert!(read(&db, "done", Some(HLC::new(20, 0))).is_empty());
    assert_eq!(read(&db, "done", None), vec!["n1"]);
    assert!(read(&db, "open", None).is_empty());
}

/// A node the build cannot place is counted, never silently dropped — the
/// callers refuse before clearing (`precheck`) and never stamp `Ready` after.
#[test]
fn build_counts_a_node_it_cannot_place() {
    let (_dir, db) = open();
    let mut lost = doc("open", 1);
    lost.path = String::new();
    store_version(&db, Some(&lost), HLC::new(10, 0));
    let wanted: build::Wanted = [("t:Doc".to_string(), defs().compound("t:Doc").to_vec())]
        .into_iter()
        .collect();
    let seen = build::write(&db, &CTX, &wanted, &HLC::new(20, 0)).unwrap();
    assert_eq!(seen.unplaceable, 1);
    assert!(build::refuse_unplaceable(&CTX, &seen).is_err());
    assert!(read(&db, "open", None).is_empty());
}
