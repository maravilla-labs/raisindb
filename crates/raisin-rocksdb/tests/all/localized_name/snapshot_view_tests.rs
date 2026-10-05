//! One lookup, one view (plan Phase 13d review): the build state that decides
//! whether the index may answer, the claims it vouches for, and the
//! fallback's child lists are all read through the lookup's snapshot.
//!
//! A rebuild commits its last batch of claims and only then stamps `Ready`.
//! A lookup whose snapshot predates both, but which read the state live,
//! paired `Ready` with a view missing the claims: a false 404 for a node the
//! index already claimed to cover.

use super::support::*;
use raisin_rocksdb::cf;
use raisin_rocksdb::localized_name::lookup::{LocalizedLookup, ServedBy};
use raisin_storage::{DeleteNodeOptions, NodeRepository, Storage};

/// Every row of the localized name index, gone — the database as a rebuild
/// sees it before its node pass has written anything.
fn wipe_index(storage: &raisin_rocksdb::RocksDBStorage) {
    let db = storage.db();
    let handle = db.cf_handle(cf::LOCALIZED_NAME_INDEX).unwrap();
    let keys: Vec<Box<[u8]>> = db
        .iterator_cf(handle, rocksdb::IteratorMode::Start)
        .map(|item| item.unwrap().0)
        .collect();
    for key in keys {
        db.delete_cf(handle, key).unwrap();
    }
}

#[tokio::test]
async fn ready_stamped_after_the_snapshot_does_not_hide_a_claim_the_snapshot_lacks() {
    let (storage, _dir) = open().await;
    let (products, chair) = catalog(&storage).await;
    wipe_index(&storage);
    assert!(!availability(&storage, B).is_ready());

    // The lookup's snapshot is taken here, before the rebuild's last batch.
    let snapshot = storage.db().snapshot();
    // The rebuild commits its claims and stamps `Ready`.
    build(&storage, B).await;
    assert!(availability(&storage, B).is_ready());
    assert_eq!(
        claimants(&storage, B, "fr", &products, "chaise"),
        [chair.clone()]
    );

    // The lookup carries on through its snapshot: the state record it reads
    // there is not `Ready`, so it takes the fallback and finds the node.
    let found = LocalizedLookup::new(storage.nodes_impl())
        .resolve_in(&snapshot, names(B), "fr", "/produits/chaise", None)
        .unwrap()
        .expect("the node exists in the lookup's view");
    assert_eq!(found.node_id, chair);
    assert_eq!(found.served_by, ServedBy::Fallback);
    drop(snapshot);

    // A fresh lookup sees the finished build.
    assert_eq!(
        id_via(
            &storage,
            B,
            "fr",
            "/produits/chaise",
            raisin_storage::localized::LocalizedServedBy::Index
        ),
        Some(chair)
    );
}

#[tokio::test]
async fn fallback_lists_children_from_the_lookups_snapshot() {
    let (storage, _dir) = open().await;
    let (_, chair) = catalog(&storage).await;
    assert!(!availability(&storage, B).is_ready(), "fallback under test");

    let snapshot = storage.db().snapshot();
    storage
        .nodes()
        .delete(scope(B), &chair, DeleteNodeOptions::default())
        .await
        .unwrap();

    // In the lookup's view the chair is there: its parent's child list must
    // come from the same view as the chair's record, not the live database.
    let found = LocalizedLookup::new(storage.nodes_impl())
        .resolve_in(&snapshot, names(B), "fr", "/produits/chaise", None)
        .unwrap()
        .expect("the chair exists in the lookup's view");
    assert_eq!(found.node_id, chair);
    assert_eq!(found.served_by, ServedBy::Fallback);
    drop(snapshot);

    assert_eq!(resolve(&storage, "fr", "/produits/chaise"), None);
}
