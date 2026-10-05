//! Phase 10b review: the `node_path` backfill must not revert an in-place
//! write that lands between deciding an entry and committing it.
//!
//! The backfill decides `NODE_PATH(R) := /a` for a legacy blob at R, but only
//! commits when its batch fills — possibly many nodes later. A
//! `versionable=false` rename in that window rewrites the node AT R (a
//! path-less blob and `NODE_PATH(R) = /b`). Committing the staged entry then
//! overwrote `/b` with the legacy blob's stale `/a`, permanently: the blob is
//! no longer legacy, so no later run looks at it again. The commit hook puts
//! the rename exactly in that window.

use crate::node_path_writer_test::{
    backfill_options, folder, id_at_path, legacy_tx_put, node_path_entries, path_at, setup, tx_put,
    BRANCH, REPO, TENANT,
};
use raisin_error::Result;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_rocksdb::management::async_indexing::repair::{
    run_repair, CommitHook, RepairKind, RepairOptions,
};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const PING: &str = "test:BackfillPing";

/// A `versionable: false` type: an update rewrites the node at its current
/// revision instead of minting one.
async fn register_non_versionable(storage: &RocksDBStorage) -> Result<()> {
    let ty = NodeType {
        id: Some(PING.to_string()),
        strict: Some(false),
        name: PING.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(false),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            ty,
            CommitMetadata::system("seed type"),
        )
        .await?;
    Ok(())
}

fn ping(path: &str) -> raisin_models::nodes::Node {
    let mut node = folder("n1", path);
    node.node_type = PING.to_string();
    node
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_place_rename_between_backfill_stage_and_commit_keeps_new_path() -> Result<()> {
    let (storage, _dir) = setup().await?;
    register_non_versionable(&storage).await?;

    // The shape pre-10b SQL writes left: a legacy blob at R embedding /a, no
    // NODE_PATH entry at R.
    let r = legacy_tx_put(&storage, &ping("/a")).await?;
    assert!(node_path_entries(&storage, "n1").is_empty());
    assert_eq!(path_at(&storage, "n1", None).await.as_deref(), Some("/a"));

    // The rename lands after the scan staged NODE_PATH(R) = /a and before the
    // commit writes it.
    let fired = Arc::new(AtomicBool::new(false));
    let hook = {
        let (storage, fired) = (storage.clone(), fired.clone());
        CommitHook(Arc::new(move || {
            if fired.swap(true, Ordering::SeqCst) {
                return;
            }
            tokio::runtime::Handle::current()
                .block_on(tx_put(&storage, &ping("/b")))
                .expect("in-place rename");
        }))
    };
    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        RepairOptions {
            before_commit: Some(hook),
            ..backfill_options()
        },
    )
    .await?;
    assert!(fired.load(Ordering::SeqCst), "the hook ran");
    let report = &reports[0];
    assert!(report.completed, "{report:?}");
    assert_eq!(report.node_path.written, 0, "{report:?}");
    assert_eq!(report.node_path.changed_during_scan, 1, "{report:?}");

    // The rename was in place: still R, and its entry survived.
    assert_eq!(
        node_path_entries(&storage, "n1"),
        vec![(r, "/b".to_string())]
    );
    assert_eq!(path_at(&storage, "n1", None).await.as_deref(), Some("/b"));
    assert_eq!(
        path_at(&storage, "n1", Some(&r)).await.as_deref(),
        Some("/b")
    );
    assert_eq!(
        id_at_path(&storage, "/b", None).await.as_deref(),
        Some("n1")
    );
    assert_eq!(id_at_path(&storage, "/a", None).await, None);
    Ok(())
}
