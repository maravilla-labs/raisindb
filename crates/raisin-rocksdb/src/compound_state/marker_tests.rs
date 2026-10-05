//! Tests for the compound state transitions in `marker.rs`.

use super::store::{read_state, CompoundStateStore};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::nodes::properties::schema::{CompoundColumnType, CompoundIndexColumn};
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState};

fn definition() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: "by_cat".to_string(),
        columns: vec![CompoundIndexColumn {
            property: "cat".to_string(),
            ascending: None,
            column_type: CompoundColumnType::String,
        }],
        has_order_column: false,
        owner: None,
    }
}

fn store() -> (CompoundStateStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage = crate::RocksDBStorage::new(dir.path()).unwrap();
    (CompoundStateStore::new(storage.db().clone()), dir)
}

fn phase(store: &CompoundStateStore) -> (CompoundBuildPhase, u64) {
    let state = read_state(&store.db, "t", "r", "main", "ws", "by_cat")
        .unwrap()
        .expect("record");
    (state.phase, state.stale_generation)
}

/// A mark that arrives while a build runs makes the build's `Ready` lose;
/// the next build, started after the mark, wins.
#[test]
fn compound_ready_cas_loses_to_newer_marker() {
    let (store, _dir) = store();
    let def = definition();
    let head = HLC::new(10, 0);

    let started = store
        .begin_build("t", "r", "main", "ws", &def, head)
        .unwrap();
    assert_ne!(started, 0, "a ticket, never the none value");
    // The first build registered a record a mark can advance.
    assert_eq!(phase(&store), (CompoundBuildPhase::Building, 0));

    // A replicated write lands mid-build.
    assert_eq!(
        store.mark_workspace_stale("t", "r", "main", "ws").unwrap(),
        1
    );

    let won = store
        .complete_build(
            "t",
            "r",
            "main",
            "ws",
            CompoundIndexState::ready(&def, head),
            started,
        )
        .unwrap();
    assert!(!won, "a Ready over a newer marker must lose");
    assert_eq!(phase(&store), (CompoundBuildPhase::NotBuilt, 1));

    let started = store
        .begin_build("t", "r", "main", "ws", &def, head)
        .unwrap();
    let won = store
        .complete_build(
            "t",
            "r",
            "main",
            "ws",
            CompoundIndexState::ready(&def, head),
            started,
        )
        .unwrap();
    assert!(won);
    assert_eq!(phase(&store), (CompoundBuildPhase::Ready, 1));
}

/// A checkpoint ingest fails every record closed, across tenants.
#[test]
fn mark_all_stale_reaches_every_record() {
    let (store, _dir) = store();
    let def = definition();
    for ws in ["a", "b"] {
        store
            .put(
                "t",
                "r",
                "main",
                ws,
                &CompoundIndexState::ready(&def, HLC::new(1, 0)),
            )
            .unwrap();
    }
    assert_eq!(store.mark_all_stale().unwrap(), 2);
    for ws in ["a", "b"] {
        let state = read_state(&store.db, "t", "r", "main", ws, "by_cat")
            .unwrap()
            .unwrap();
        assert_eq!(state.phase, CompoundBuildPhase::NotBuilt);
        assert_eq!(state.stale_generation, 1);
    }
}

/// A rebuild that clears the keyspace goes `Building` at the CURRENT
/// generation, and a mark that lands mid-rebuild beats its `Ready`.
#[test]
fn rebuild_loses_to_mark_that_lands_mid_rebuild() {
    let (store, _dir) = store();
    let def = definition();
    let head = HLC::new(10, 0);
    store
        .put(
            "t",
            "r",
            "main",
            "ws",
            &CompoundIndexState::ready(&def, head),
        )
        .unwrap();
    store.mark_workspace_stale("t", "r", "main", "ws").unwrap();

    let started = store
        .begin_rebuild("t", "r", "main", "ws", &def, head)
        .unwrap();
    assert_eq!(phase(&store), (CompoundBuildPhase::Building, 1));
    store.mark_workspace_stale("t", "r", "main", "ws").unwrap();
    let ready = CompoundIndexState::ready(&def, head);
    assert!(!store
        .complete_build("t", "r", "main", "ws", ready, started)
        .unwrap());
    assert_eq!(phase(&store), (CompoundBuildPhase::NotBuilt, 2));
}

/// Two builders of one index interleave (the admin `REBUILD … compound` did
/// not take the keyspace lock): the first registers, the second registers and
/// clears the keyspace the first was writing. Registering does not advance
/// the generation, so a generation compare-and-set let BOTH stamp `Ready` —
/// one of them over a keyspace the other had half cleared. The ticket lets
/// only the LAST registration stamp.
#[test]
fn interleaved_rebuilds_cannot_both_stamp_ready() {
    let (store, _dir) = store();
    let def = definition();
    let head = HLC::new(10, 0);
    let chain = store
        .begin_rebuild("t", "r", "main", "ws", &def, head)
        .unwrap();
    let admin = store
        .begin_rebuild("t", "r", "main", "ws", &def, head)
        .unwrap();
    assert_ne!(chain, admin);
    let ready = || CompoundIndexState::ready(&def, head);
    assert!(
        !store
            .complete_build("t", "r", "main", "ws", ready(), chain)
            .unwrap(),
        "the earlier registration must not stamp Ready over the later build's clear"
    );
    assert_eq!(phase(&store), (CompoundBuildPhase::Building, 0));
    assert!(store
        .complete_build("t", "r", "main", "ws", ready(), admin)
        .unwrap());
    assert_eq!(phase(&store), (CompoundBuildPhase::Ready, 0));
}

/// A drop deletes the record (generation back to 0) while a build that began
/// under generation 0 is still running; a later build recreates the record
/// at generation 0. The old build must not match the fresh record.
#[test]
fn a_dropped_and_recreated_record_does_not_revive_an_old_build() {
    let (store, _dir) = store();
    let def = definition();
    let head = HLC::new(10, 0);
    let old = store
        .begin_rebuild("t", "r", "main", "ws", &def, head)
        .unwrap();
    let cf = crate::cf_handle(&store.db, crate::cf::INDEX_STATUS).unwrap();
    store
        .db
        .delete_cf(
            cf,
            super::compound_state_key("t", "r", "main", "ws", "by_cat"),
        )
        .unwrap();
    let fresh = store
        .begin_rebuild("t", "r", "main", "ws", &def, head)
        .unwrap();
    assert_eq!(phase(&store), (CompoundBuildPhase::Building, 0));
    assert!(!store
        .complete_build(
            "t",
            "r",
            "main",
            "ws",
            CompoundIndexState::ready(&def, head),
            old
        )
        .unwrap());
    assert!(store
        .complete_build(
            "t",
            "r",
            "main",
            "ws",
            CompoundIndexState::ready(&def, head),
            fresh
        )
        .unwrap());
}

/// A checkpoint ingest put-merges a PEER's record over a local build's
/// `Building` record, and `mark_all_stale` then sets the peer's generation
/// + 1 — which can equal the generation the local build started under. The
/// build must still lose: its scan predates the ingest.
#[test]
fn a_peer_record_put_over_a_build_by_an_ingest_never_matches() {
    let (store, _dir) = store();
    let def = definition();
    let head = HLC::new(10, 0);
    // Local generation 1 (marked once before), and a build under it.
    store
        .put(
            "t",
            "r",
            "main",
            "ws",
            &CompoundIndexState::ready(&def, head),
        )
        .unwrap();
    store.mark_workspace_stale("t", "r", "main", "ws").unwrap();
    let local = store
        .begin_rebuild("t", "r", "main", "ws", &def, head)
        .unwrap();
    assert_eq!(phase(&store), (CompoundBuildPhase::Building, 1));
    // The ingest: the peer's Ready at generation 0, written verbatim (as the
    // SST copy does, bypassing the store), then failed closed.
    let peer = CompoundIndexState::ready(&def, head);
    let cf = crate::cf_handle(&store.db, crate::cf::INDEX_STATUS).unwrap();
    store
        .db
        .put_cf(
            cf,
            super::compound_state_key("t", "r", "main", "ws", "by_cat"),
            rmp_serde::to_vec(&peer).unwrap(),
        )
        .unwrap();
    store.mark_all_stale().unwrap();
    assert_eq!(phase(&store), (CompoundBuildPhase::NotBuilt, 1));
    assert!(
        !store
            .complete_build(
                "t",
                "r",
                "main",
                "ws",
                CompoundIndexState::ready(&def, head),
                local
            )
            .unwrap(),
        "a build whose scan predates the ingest must not stamp Ready"
    );
    assert_eq!(phase(&store), (CompoundBuildPhase::NotBuilt, 1));
}
