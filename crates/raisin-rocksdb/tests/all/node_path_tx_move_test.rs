//! Phase 10 review: a write and an ancestor move in ONE transaction, and the
//! repairs that read a node's path.
//!
//! `put_node` and `move_node_tree` share the transaction revision R. A blob
//! and a `NODE_PATH` entry at R naming different paths is a tie the read rule
//! must not settle by trusting either side; the move must not leave one
//! behind, and the backfill / PATH_INDEX repair must not cement one.

use crate::node_path_writer_test::{
    backfill_options, blob_at, folder, head, id_at_path, legacy_tx_put, node_path_entries, path_at,
    repo_create, scope, setup, BRANCH, REPO, TENANT, WS,
};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind};
use raisin_rocksdb::management::async_indexing::repair_path_index;
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{NodeRepository, Storage};

/// Committed `/a`, `/a/k`, `/a/k/c` and `/x`, created through the repository.
async fn tree(storage: &RocksDBStorage) -> Result<()> {
    for (id, path) in [("a", "/a"), ("k", "/a/k"), ("c", "/a/k/c"), ("x", "/x")] {
        repo_create(storage, folder(id, path)).await?;
    }
    Ok(())
}

async fn begin(storage: &RocksDBStorage) -> Result<Box<dyn TransactionalContext>> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("tx")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    Ok(tx)
}

/// `c`'s title property written through the transaction's own read of it.
async fn retitle(tx: &dyn TransactionalContext, title: &str) -> Result<String> {
    let mut c = tx.get_node(WS, "c").await?.expect("c");
    c.properties
        .insert("title".into(), PropertyValue::String(title.into()));
    tx.put_node(WS, &c).await?;
    Ok(c.path)
}

/// Every reader agrees `c` sits at `/x/a/k/c` — by id at HEAD and at the
/// commit revision (what the replication capture reads), by path, and in the
/// moved subtree's listing — and its record keeps the title.
async fn assert_moved(storage: &RocksDBStorage, rev: &HLC, title: &str) -> Result<()> {
    let want = Some("/x/a/k/c");
    assert_eq!(path_at(storage, "c", None).await.as_deref(), want);
    assert_eq!(path_at(storage, "c", Some(rev)).await.as_deref(), want);
    assert_eq!(
        id_at_path(storage, "/x/a/k/c", None).await.as_deref(),
        Some("c")
    );
    assert_eq!(id_at_path(storage, "/a/k/c", None).await, None);
    let subtree = storage
        .nodes()
        .get_descendants_bulk(scope(), "/x/a", u32::MAX, None)
        .await?;
    assert!(subtree.contains_key("/x/a/k/c"), "{:?}", subtree.keys());
    let c = storage.nodes().get(scope(), "c", None).await?.expect("c");
    assert_eq!(
        c.properties.get("title"),
        Some(&PropertyValue::String(title.into()))
    );

    // The record at R is the one format: a path-less blob, and NODE_PATH at
    // R naming the moved path.
    let (decoded, _) = raisin_rocksdb::decode_node_blob(&blob_at(storage, "c", rev))?;
    assert!(
        decoded.path.is_empty(),
        "the blob at R embeds {:?}",
        decoded.path
    );
    let at_r: Vec<_> = node_path_entries(storage, "c")
        .into_iter()
        .filter(|(r, _)| r == rev)
        .collect();
    assert_eq!(at_r, vec![(*rev, "/x/a/k/c".to_string())]);

    // And the backfill finds nothing to fight over.
    let report = run_repair(
        storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        backfill_options(),
    )
    .await?;
    assert_eq!(report[0].node_path.conflicts, 0, "{report:?}");
    assert_eq!(path_at(storage, "c", None).await.as_deref(), want);
    Ok(())
}

/// `put_node(c)` then `move_node_tree(/a -> /x/a)` in one transaction. Before
/// Phase 10 the move wrote NODE_PATH /x/a/k/c at R beside the put's legacy
/// blob /a/k/c at R, and the tie went to the blob: c read back at a parent
/// that no longer exists, and the replicated snapshot carried it to every
/// peer. (That legacy shape, written raw, is
/// `node_path_backfill_never_overwrites_a_same_revision_entry`.)
#[tokio::test]
async fn put_then_ancestor_move_in_one_tx_reads_moved_path() -> Result<()> {
    let (storage, _dir) = setup().await?;
    tree(&storage).await?;
    let tx = begin(&storage).await?;
    retitle(tx.as_ref(), "v2").await?;
    tx.move_node_tree(WS, "a", "/x/a").await?;
    tx.commit().await?;
    let rev = head(&storage).await?;
    assert_moved(&storage, &rev, "v2").await
}

/// `move_node_tree(/a -> /x/a)` then `put_node(c)` in one transaction. The
/// move never updated c's cached record, so the in-transaction read returned
/// the pre-move path and the put stored it back at R.
#[tokio::test]
async fn ancestor_move_then_put_in_one_tx_keeps_moved_path() -> Result<()> {
    let (storage, _dir) = setup().await?;
    tree(&storage).await?;
    let tx = begin(&storage).await?;
    tx.move_node_tree(WS, "a", "/x/a").await?;
    let seen = retitle(tx.as_ref(), "v3").await?;
    assert_eq!(seen, "/x/a/k/c", "the in-transaction read after the move");
    tx.commit().await?;
    let rev = head(&storage).await?;
    assert_moved(&storage, &rev, "v3").await
}

fn raw_put(storage: &RocksDBStorage, cf_name: &str, key: Vec<u8>, value: &[u8]) {
    let db = storage.db();
    db.put_cf(db.cf_handle(cf_name).unwrap(), key, value)
        .unwrap();
}

/// What a pre-Phase-10 binary left for put-then-move in one transaction:
/// legacy blob /a/k/c at R, NODE_PATH /x/a/k/c at R, PATH_INDEX moved at R.
/// The read rule must read /x/a/k/c (the old "NODE_PATH wins" did), and the
/// backfill must not overwrite the entry at R with the blob's stale path.
#[tokio::test]
async fn node_path_backfill_never_overwrites_a_same_revision_entry() -> Result<()> {
    let (storage, _dir) = setup().await?;
    tree(&storage).await?;
    let base = head(&storage).await?;
    let r = HLC::new(base.timestamp_ms + 1_000, 0);
    let legacy = rmp_serde::to_vec_named(&folder("c", "/a/k/c")).unwrap();
    raw_put(
        &storage,
        cf::NODES,
        keys::node_key_versioned(TENANT, REPO, BRANCH, WS, "c", &r),
        &legacy,
    );
    raw_put(
        &storage,
        cf::NODE_PATH,
        keys::node_path_key_versioned(TENANT, REPO, BRANCH, WS, "c", &r),
        b"/x/a/k/c",
    );
    raw_put(
        &storage,
        cf::PATH_INDEX,
        keys::path_index_key_versioned(TENANT, REPO, BRANCH, WS, "/a/k/c", &r),
        keys::TOMBSTONE_VALUE,
    );
    raw_put(
        &storage,
        cf::PATH_INDEX,
        keys::path_index_key_versioned(TENANT, REPO, BRANCH, WS, "/x/a/k/c", &r),
        b"c",
    );
    assert_eq!(
        path_at(&storage, "c", Some(&r)).await.as_deref(),
        Some("/x/a/k/c")
    );

    let report = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        backfill_options(),
    )
    .await?;
    assert_eq!(report[0].node_path.conflicts, 1, "{report:?}");
    assert_eq!(report[0].node_path.written, 0, "{report:?}");
    assert_eq!(
        node_path_entries(&storage, "c").last(),
        Some(&(r, "/x/a/k/c".to_string()))
    );
    assert_eq!(
        path_at(&storage, "c", Some(&r)).await.as_deref(),
        Some("/x/a/k/c")
    );
    Ok(())
}

/// Clear PATH_INDEX the way the rebuild incident did, then repair. Taking
/// NODE_PATH alone restored a renamed node at its OLD path (a phantom) and
/// skipped a node only the transaction path had written.
#[tokio::test]
async fn repair_path_index_restores_paths_by_the_read_rule() -> Result<()> {
    let (storage, _dir) = setup().await?;
    repo_create(&storage, folder("n1", "/p1")).await?;
    legacy_tx_put(&storage, &folder("n1", "/p2")).await?; // legacy rename
    legacy_tx_put(&storage, &folder("t", "/t")).await?; // legacy create, no entry

    let db = storage.db();
    let cf_path = db.cf_handle(cf::PATH_INDEX).unwrap();
    let prefix = keys::branch_prefix(TENANT, REPO, BRANCH);
    let doomed: Vec<_> = db
        .prefix_iterator_cf(cf_path, &prefix)
        .flatten()
        .take_while(|(k, _)| k.starts_with(&prefix))
        .map(|(k, _)| k)
        .collect();
    assert!(!doomed.is_empty());
    for key in doomed {
        db.delete_cf(cf_path, key).unwrap();
    }
    assert_eq!(id_at_path(&storage, "/p2", None).await, None);

    let stats = repair_path_index(&storage, TENANT, REPO, BRANCH, WS, false).await?;
    assert_eq!(stats.nodes_seen, 2, "{stats:?}");
    assert_eq!(stats.entries_written, 2, "{stats:?}");
    assert_eq!(
        id_at_path(&storage, "/p2", None).await.as_deref(),
        Some("n1")
    );
    assert_eq!(id_at_path(&storage, "/p1", None).await, None, "no phantom");
    assert_eq!(id_at_path(&storage, "/t", None).await.as_deref(), Some("t"));
    Ok(())
}

/// `move_node_tree(/a/k -> /x/k)` then `add_node(/a/k)` in one transaction.
/// The create's own path check saw the vacated path, but the shared create
/// validation re-checked COMMITTED state, still found `k` at /a/k and
/// rejected the create ("Node with path '/a/k' already exists"). Found by
/// `mvcc_index_oracle::stages::oracle_replication_replay`.
#[tokio::test]
async fn move_out_then_create_at_vacated_path_in_one_tx() -> Result<()> {
    let (storage, _dir) = setup().await?;
    tree(&storage).await?;
    let tx = begin(&storage).await?;
    tx.move_node_tree(WS, "k", "/x/k").await?;
    tx.add_node(WS, &folder("n", "/a/k")).await?;
    tx.commit().await?;

    assert_eq!(
        id_at_path(&storage, "/a/k", None).await.as_deref(),
        Some("n")
    );
    assert_eq!(
        id_at_path(&storage, "/x/k", None).await.as_deref(),
        Some("k")
    );
    assert_eq!(
        path_at(&storage, "c", None).await.as_deref(),
        Some("/x/k/c")
    );
    assert_eq!(id_at_path(&storage, "/a/k/c", None).await, None);
    Ok(())
}
