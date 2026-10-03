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
        owner_node_type: None,
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
    assert_eq!(started, 0);
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
    assert_eq!(started, 1);
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
