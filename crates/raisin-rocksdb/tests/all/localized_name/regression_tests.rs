//! Regressions from the Phase 12 review: commits, rebuilds, build state and
//! the write-path cost, each named after the failure it pins.

use super::support::*;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleOverlay};
use raisin_rocksdb::cf;
use raisin_rocksdb::localized_name::{state, Availability};
use raisin_storage::localized::LocalizedServedBy::Index;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{BranchRepository, NodeRepository, Storage};
use std::collections::{BTreeMap, HashMap};

fn name_overlay(name: &str) -> LocaleOverlay {
    LocaleOverlay::properties(HashMap::from([(
        JsonPointer::new("/__node_name"),
        PropertyValue::String(name.to_string()),
    )]))
}

async fn tx(storage: &raisin_rocksdb::RocksDBStorage) -> Box<dyn TransactionalContext> {
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_validate_schema(false).unwrap();
    tx
}

fn index_keys(storage: &raisin_rocksdb::RocksDBStorage) -> usize {
    let db = storage.db();
    db.iterator_cf(
        db.cf_handle(cf::LOCALIZED_NAME_INDEX).unwrap(),
        rocksdb::IteratorMode::Start,
    )
    .count()
}

/// Two commits of one node staged from the same pre-commit rows: T1 (older
/// revision) renames `chaise` -> `siege`, T2 (newer) writes `chaise` back
/// from a stale read. T2 commits first. Staged outside the branch lock, T2
/// wrote nothing (unchanged against the rows it read) and T1 then killed
/// `chaise` at its revision — HEAD said `chaise` while the index said
/// `siege`, a 404 through a `Ready` index.
#[tokio::test]
async fn concurrent_commits_keep_the_head_name() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (_, chair) = catalog(&storage).await;

    let t1 = tx(&storage).await;
    t1.store_translation(WS, &chair, "fr", name_overlay("siege"))
        .await
        .unwrap();
    let t2 = tx(&storage).await;
    t2.store_translation(WS, &chair, "fr", name_overlay("chaise"))
        .await
        .unwrap();
    t2.commit().await.unwrap();
    t1.commit().await.unwrap();

    assert!(availability(&storage, B).is_ready());
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(chair)
    );
    assert_eq!(resolve(&storage, "fr", "/produits/siege"), None);
}

/// A move and a rename in ONE transaction: the record writer saw the new
/// node with the old overlay, the overlay writer the old node with the new
/// overlay; the commit's final-view full put decides.
#[tokio::test]
async fn one_transaction_move_and_rename_serves_the_final_name() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (_, chair) = catalog(&storage).await;
    let archive = create(&storage, "/archive", &[]).await;
    set_name(&storage, &archive, "fr", "archives").await;

    let t = tx(&storage).await;
    t.move_node_tree(WS, &chair, "/archive/chair")
        .await
        .unwrap();
    t.store_translation(WS, &chair, "fr", name_overlay("siege"))
        .await
        .unwrap();
    t.commit().await.unwrap();

    assert_eq!(
        id_via(&storage, B, "fr", "/archives/siege", Index),
        Some(chair)
    );
    assert_eq!(resolve(&storage, "fr", "/produits/chaise"), None);
    assert_eq!(resolve(&storage, "fr", "/archives/chaise"), None);
}

/// Raw `{t}\0{r}\0{b}\0{ws}\0…\0{~rev}` key of the index.
fn index_key(parts: &[&str], revision: &HLC) -> Vec<u8> {
    let mut key = Vec::new();
    for part in [T, R, B, WS].iter().chain(parts) {
        key.extend_from_slice(part.as_bytes());
        key.push(0);
    }
    key.extend_from_slice(&revision.encode_descending());
    key
}

/// A row NEWER than every live input of a node, at or below the pin (a
/// catch-up, a merge's full put, a checkpoint peer's): the rebuild used to
/// write at the newest live input and diff below that row, saw "unchanged",
/// skipped the node — and stamped `Ready` over a HEAD that said "no name".
#[tokio::test]
async fn rebuild_over_a_newer_stored_row_keeps_the_rebuilt_claim() {
    let (storage, _dir) = open().await;
    let (products, chair) = catalog(&storage).await;
    let named_at = head(&storage, B).await;
    let peer_row = HLC {
        timestamp_ms: named_at.timestamp_ms,
        counter: named_at.counter + 1,
    };
    let db = storage.db();
    let cf = db.cf_handle(cf::LOCALIZED_NAME_INDEX).unwrap();
    db.put_cf(cf, index_key(&["lname_of", &chair, "fr"], &peer_row), b"T")
        .unwrap();
    db.put_cf(
        cf,
        index_key(&["lname", "fr", &products, "chaise", &chair], &peer_row),
        b"T",
    )
    .unwrap();
    create(&storage, "/other", &[]).await; // the pin lies above the row

    build(&storage, B).await;
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(chair)
    );
}

/// A config A -> B -> A, or a checkpoint ingest, while a build runs: a newer
/// run began `Building` under the SAME fingerprint and pin, and the older run
/// finishing first used to stamp `Ready` over it.
#[tokio::test]
async fn stale_build_cannot_stamp_ready_after_a_newer_begin() {
    let (storage, _dir) = open().await;
    let db = storage.db();
    let pin = head(&storage, B).await;
    let workspaces = [WS.to_string()];
    let first = state::begin_build(db, (T, R, B), &workspaces, "fp", &pin).unwrap();
    state::mark_all_not_built(db).unwrap();
    let second = state::begin_build(db, (T, R, B), &workspaces, "fp", &pin).unwrap();
    assert!(second > first, "every run gets its own generation");
    let none = BTreeMap::new();
    assert!(state::finish_build(db, (T, R, B), "fp", &pin, first, &none)
        .unwrap()
        .is_empty());
    assert_eq!(
        state::finish_build(db, (T, R, B), "fp", &pin, second, &none).unwrap(),
        vec![WS.to_string()]
    );
}

/// A node with a name to claim whose parent cannot be resolved (here a
/// transactional orphan with its overlay in the same batch) used to be
/// skipped with a warning, leaving a `Ready` index without it.
#[tokio::test]
async fn unresolvable_parent_invalidates_the_build() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    assert!(availability(&storage, B).is_ready());
    let orphan = page("/ghost/child", &[]);
    let t = tx(&storage).await;
    t.add_node(WS, &orphan).await.unwrap();
    t.store_translation(WS, &orphan.id, "fr", name_overlay("enfant"))
        .await
        .unwrap();
    t.commit().await.unwrap();
    assert_eq!(availability(&storage, B), Availability::NotBuilt);
}

/// A branch deleted and re-created under its name is a fork, never built
/// here; it used to inherit the deleted branch's `Ready`.
#[tokio::test]
async fn deleted_branch_recreated_is_not_ready() {
    let (storage, _dir) = open().await;
    let branches = storage.branches();
    branches
        .create_branch(T, R, "preview", "t", None, Some(B.into()), false, false)
        .await
        .unwrap();
    build(&storage, "preview").await;
    assert!(availability(&storage, "preview").is_ready());
    branches.delete_branch(T, R, "preview").await.unwrap();
    branches
        .create_branch(T, R, "preview", "t", None, Some(B.into()), false, false)
        .await
        .unwrap();
    assert_eq!(availability(&storage, "preview"), Availability::NotBuilt);
}

/// Every record write paid for the index — config decode, overlay and row
/// scans, a full blob decode for the catch-up — even where nothing can have
/// a name. Untranslated writes now stop at two key probes and write nothing.
#[tokio::test]
async fn untranslated_writes_touch_no_index_rows() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let products = create(&storage, "/products", &[]).await;
    let chair = create(&storage, "/products/chair", &[("title", "v1")]).await;
    update_props(&storage, &chair, &[("title", "v2")]).await;
    create(&storage, "/archive", &[]).await;
    storage
        .nodes()
        .move_node(scope(B), &chair, "/archive/chair", None)
        .await
        .unwrap();
    assert_eq!(index_keys(&storage), 0);
    assert!(availability(&storage, B).is_ready());
    let _ = products;
}
