//! The indexed lifeline answers exactly what the `NODES` walk answers, over
//! random histories — deletes, re-creates, edits, stale entries (GC'd or
//! overwritten tombstones), overlay versions at every revision, reads at past
//! bounds and at HEAD — and readiness gates which one a read takes.

use super::state::{self, IndexStatus};
use crate::mvcc_read::{DbRead, NodeLifeline, SnapshotRead};
use crate::{cf, cf_handle, keys};
use raisin_hlc::HLC;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rocksdb::DB;

const T: &str = "t";
const R: &str = "r";
const B: &str = "main";
const WS: &str = "ws";
const ID: &str = "node-1";
const SCOPE: (&str, &str, &str, &str) = (T, R, B, WS);

fn rev(n: u64) -> HLC {
    HLC::new(1_000 + n, 0)
}

fn storage() -> (tempfile::TempDir, crate::RocksDBStorage) {
    let dir = tempfile::tempdir().unwrap();
    let storage = crate::RocksDBStorage::new(dir.path()).unwrap();
    (dir, storage)
}

fn put_record(db: &DB, at: u64, tombstone: bool) {
    let key = keys::node_key_versioned(T, R, B, WS, ID, &rev(at));
    let value: &[u8] = if tombstone { b"T" } else { b"node-blob" };
    db.put_cf(cf_handle(db, cf::NODES).unwrap(), key, value)
        .unwrap();
}

fn put_entry(db: &DB, at: u64) {
    db.put_cf(
        cf_handle(db, cf::NODE_DELETES).unwrap(),
        super::entry_key(T, R, B, WS, ID, &rev(at)),
        b"",
    )
    .unwrap();
}

fn make_ready(db: &DB) {
    let (generation, _) = state::begin_build(db, (T, R, B), None).unwrap();
    assert!(state::finish_build(db, (T, R, B), generation).unwrap());
}

fn walked(db: &DB, from: u64, to: Option<u64>, version: u64) -> bool {
    let to = to.map(rev);
    let mut src = DbRead(db);
    let lifeline = NodeLifeline::walked_in(&mut src, SCOPE, ID, &rev(from), to.as_ref()).unwrap();
    assert!(!lifeline.is_indexed());
    lifeline.ends_in(&mut src, &rev(version)).unwrap()
}

/// One random history: `len` revisions, each a live record, a tombstone or
/// nothing; every tombstone has its entry (completeness), plus stale entries.
fn random_history(db: &DB, rng: &mut StdRng, len: u64) {
    for at in 1..=len {
        match rng.gen_range(0..10) {
            0..=4 => put_record(db, at, false), // create / edit / re-create
            5..=6 => {
                put_record(db, at, true);
                put_entry(db, at);
            }
            7 => {
                // A tombstone GC dropped (or a record overwritten in place):
                // the entry stays, NODES holds a live record or nothing.
                if rng.gen_bool(0.5) {
                    put_record(db, at, false);
                }
                put_entry(db, at);
            }
            _ => {} // a revision of some other node
        }
    }
}

#[test]
fn the_index_answers_like_the_walk_over_random_histories() {
    let mut checked = 0u64;
    for seed in 0..60u64 {
        let (_dir, storage) = storage();
        let db = storage.db();
        let mut rng = StdRng::seed_from_u64(seed);
        let len = rng.gen_range(1..40);
        random_history(db, &mut rng, len);
        make_ready(db);
        for _ in 0..40 {
            let from = rng.gen_range(1..=len);
            let to = if rng.gen_bool(0.3) {
                None
            } else {
                Some(rng.gen_range(from..=len + 2))
            };
            let top = to.unwrap_or(len + 2);
            // Two sources: the live database and a pinned snapshot.
            let snapshot = db.snapshot();
            let mut pinned = SnapshotRead::new(db, &snapshot);
            let mut live = DbRead(db);
            let indexed =
                NodeLifeline::read_in(&mut pinned, SCOPE, ID, &rev(from), to.map(rev).as_ref())
                    .unwrap();
            let indexed_live =
                NodeLifeline::read_in(&mut live, SCOPE, ID, &rev(from), to.map(rev).as_ref())
                    .unwrap();
            assert!(indexed.is_indexed() && indexed_live.is_indexed());
            for version in from..=top {
                let want = walked(db, from, to, version);
                assert_eq!(
                    indexed.ends_in(&mut pinned, &rev(version)).unwrap(),
                    want,
                    "seed {seed}: from {from} to {to:?} version {version}"
                );
                assert_eq!(
                    indexed_live.ends_in(&mut live, &rev(version)).unwrap(),
                    want,
                    "seed {seed} (live): from {from} to {to:?} version {version}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 10_000, "only {checked} cases");
}

#[test]
fn a_node_never_deleted_costs_one_seek() {
    let (_dir, storage) = storage();
    let db = storage.db();
    for at in 1..=100 {
        put_record(db, at, false);
    }
    make_ready(db);
    let snapshot = db.snapshot();
    let mut src = SnapshotRead::new(db, &snapshot);
    let lifeline = NodeLifeline::read_in(&mut src, SCOPE, ID, &rev(1), None).unwrap();
    for version in 1..=100 {
        assert!(!lifeline.ends_in(&mut src, &rev(version)).unwrap());
    }
    // The readiness record is a point get; the only seek is the entry scan.
    assert_eq!(src.seeks(), 1);
}

#[test]
fn reads_walk_until_the_branch_is_ready() {
    let (_dir, storage) = storage();
    let db = storage.db();
    put_record(db, 1, false);
    // A tombstone with NO entry: a database from before the index.
    put_record(db, 5, true);
    let read = |db: &DB| NodeLifeline::read(db, SCOPE, ID, &rev(1), None).unwrap();
    assert!(!read(db).is_indexed(), "no record: walk");
    assert!(
        read(db).ends(db, &rev(1)).unwrap(),
        "the walk sees the delete"
    );

    let (generation, _) = state::begin_build(db, (T, R, B), None).unwrap();
    assert!(!read(db).is_indexed(), "building: walk");
    // A backfill would write the entry; without it a READY index would be
    // wrong — which is exactly what readiness exists to prevent.
    put_entry(db, 5);
    assert!(state::finish_build(db, (T, R, B), generation).unwrap());
    assert!(read(db).is_indexed());
    assert!(read(db).ends(db, &rev(1)).unwrap());
}

#[test]
fn an_invalidation_during_a_build_keeps_the_branch_not_ready() {
    let (_dir, storage) = storage();
    let db = storage.db();
    let (generation, continued) = state::begin_build(db, (T, R, B), None).unwrap();
    assert!(!continued);
    state::mark_all_not_built(db).unwrap();
    assert!(!state::finish_build(db, (T, R, B), generation).unwrap());
    assert!(!state::is_ready(db, T, R, B));

    // A resumed build continues its generation only while nothing changed.
    let (g2, _) = state::begin_build(db, (T, R, B), None).unwrap();
    assert!(g2 > generation);
    assert_eq!(
        state::begin_build(db, (T, R, B), Some(g2)).unwrap(),
        (g2, true)
    );
    state::invalidate(db, (T, R, B)).unwrap();
    let (g3, continued) = state::begin_build(db, (T, R, B), Some(g2)).unwrap();
    assert!(!continued && g3 > g2);
    assert!(state::finish_build(db, (T, R, B), g3).unwrap());
    assert!(state::is_ready(db, T, R, B));
}

#[test]
fn a_build_cannot_finish_while_a_copy_writes_into_the_branch() {
    let (_dir, storage) = storage();
    let db = storage.db();
    make_ready(db);
    // Source and target both ready: the copy keeps the target ready.
    let ticket = state::before_branch_copy(db, T, R, B, B).unwrap();
    assert!(!state::is_ready(db, T, R, B), "not trusted during the copy");
    let (generation, _) = state::begin_build(db, (T, R, B), None).unwrap();
    assert!(
        !state::finish_build(db, (T, R, B), generation).unwrap(),
        "a backfill that ran during the copy cannot vouch for it"
    );
    // The build bumped the generation: the ticket no longer holds.
    assert!(!state::after_branch_copy(db, T, R, B, B, ticket).unwrap());
    assert!(!state::is_ready(db, T, R, B));

    let other = "feature";
    make_ready(db);
    let (g, _) = state::begin_build(db, (T, R, other), None).unwrap();
    assert!(state::finish_build(db, (T, R, other), g).unwrap());
    let ticket = state::before_branch_copy(db, T, R, B, other).unwrap();
    assert!(state::after_branch_copy(db, T, R, B, other, ticket).unwrap());
    assert!(state::is_ready(db, T, R, other));
    assert_eq!(
        state::read(db, (T, R, other)).unwrap().unwrap().status,
        IndexStatus::Ready
    );
}
