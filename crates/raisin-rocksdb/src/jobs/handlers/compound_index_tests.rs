//! The compound build lease is per NODE.

use super::*;
use crate::repositories::{BranchRepositoryImpl, RevisionRepositoryImpl};
use raisin_models::nodes::Node;
use raisin_storage::compound::CompoundBuildPhase;
use raisin_storage::jobs::{JobId, JobStatus};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CompoundIndexRepository, CreateNodeOptions,
    NodeRepository, NodeTypeRepository, Storage, StorageScope,
};
use std::collections::HashMap;

const T: &str = "t";
const R: &str = "r";
const B: &str = "main";
const WS: &str = "ws";
const INDEX: &str = "by_cat";

async fn setup() -> (crate::RocksDBStorage, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage = crate::RocksDBStorage::new(dir.path()).unwrap();
    storage
        .branches()
        .create_branch(T, R, B, "system", None, None, false, false)
        .await
        .unwrap();
    let node_type = serde_json::from_value(serde_json::json!({
        "name": "test:Item",
        "compound_indexes": [{
            "name": INDEX,
            "columns": [{ "property": "cat", "column_type": "String" }],
            "has_order_column": false
        }]
    }))
    .unwrap();
    storage
        .node_types()
        .create(
            BranchScope::new(T, R, B),
            node_type,
            CommitMetadata::system("types"),
        )
        .await
        .unwrap();
    for i in 0..3 {
        let node = Node {
            id: format!("n{i}"),
            name: format!("n{i}"),
            path: format!("/n{i}"),
            node_type: "test:Item".to_string(),
            properties: HashMap::from([(
                "cat".to_string(),
                raisin_models::nodes::properties::PropertyValue::String("a".to_string()),
            )]),
            created_at: Some(chrono::Utc::now()),
            ..Node::default()
        };
        storage
            .nodes()
            .create(
                StorageScope::new(T, R, B, WS),
                node,
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    operation_meta: None,
                },
            )
            .await
            .unwrap();
    }
    (storage, dir)
}

fn handler(
    storage: &crate::RocksDBStorage,
    locks: &raisin_locks::LockManagerHandle,
    node_id: &str,
) -> CompoundIndexJobHandler {
    let db = storage.db().clone();
    CompoundIndexJobHandler::new(
        db.clone(),
        Arc::new(RevisionRepositoryImpl::new(db.clone(), node_id.to_string())),
        Arc::new(BranchRepositoryImpl::new(db)),
    )
    .with_lock_manager(Some(locks.clone()))
    .with_node_id(node_id)
}

fn job() -> (JobInfo, JobContext) {
    let job = JobInfo {
        id: JobId("compound-build".to_string()),
        job_type: JobType::CompoundIndexBuild {
            tenant_id: T.to_string(),
            repo_id: R.to_string(),
            branch: B.to_string(),
            workspace: WS.to_string(),
            node_type_name: "test:Item".to_string(),
            index_name: INDEX.to_string(),
        },
        status: JobStatus::Running,
        tenant: T.to_string(),
        started_at: chrono::Utc::now(),
        completed_at: None,
        progress: None,
        error: None,
        result: None,
        retry_count: 0,
        max_retries: 0,
        last_heartbeat: None,
        timeout_seconds: 300,
        next_retry_at: None,
        executing_since: None,
    };
    let context = JobContext {
        tenant_id: T.to_string(),
        repo_id: R.to_string(),
        branch: B.to_string(),
        workspace_id: WS.to_string(),
        revision: HLC::new(0, 0),
        metadata: HashMap::new(),
    };
    (job, context)
}

fn state(storage: &crate::RocksDBStorage) -> Option<CompoundBuildPhase> {
    crate::compound_state::read_state(storage.db(), T, R, B, WS, INDEX)
        .unwrap()
        .map(|s| s.phase)
}

fn lease_key(node_id: &str) -> String {
    raisin_locks::scoped_key(
        T,
        R,
        B,
        &format!("compound-index-build:{node_id}:{WS}:{INDEX}"),
    )
}

/// The origin holding ITS lease (mid-build) must not stop a replica from
/// building its own local keyspace. With a cluster-wide key it did: the
/// replica skipped with "being built elsewhere" and stayed `NotBuilt`.
#[tokio::test]
async fn compound_build_runs_on_replica_while_origin_holds_lease() {
    let (storage, _dir) = setup().await;
    let locks: raisin_locks::LockManagerHandle =
        Arc::new(raisin_locks::InProcessLockManager::new());

    let held = locks
        .try_acquire(&lease_key("origin"), "origin", BUILD_LEASE_TTL)
        .await
        .unwrap();
    assert!(held.is_some(), "origin holds its lease");

    let (job, context) = job();
    handler(&storage, &locks, "replica")
        .handle(&job, &context)
        .await
        .unwrap();
    assert_eq!(state(&storage), Some(CompoundBuildPhase::Ready));
}

/// The lease still serializes builds on ONE node: a second build of the same
/// index there is skipped while the first holds the lease.
#[tokio::test]
async fn compound_build_is_still_serialized_on_one_node() {
    let (storage, _dir) = setup().await;
    let locks: raisin_locks::LockManagerHandle =
        Arc::new(raisin_locks::InProcessLockManager::new());

    let held = locks
        .try_acquire(&lease_key("replica"), "replica", BUILD_LEASE_TTL)
        .await
        .unwrap();
    assert!(held.is_some());

    let (job, context) = job();
    handler(&storage, &locks, "replica")
        .handle(&job, &context)
        .await
        .unwrap();
    assert_eq!(
        state(&storage),
        None,
        "the second build on one node must skip"
    );
}

/// A node the build cannot place (no NODE_PATH) must not be dropped from an
/// index the build then stamps `Ready`: the build refuses BEFORE it clears,
/// so the entries that exist survive and the index stays unusable.
#[tokio::test]
async fn compound_build_refuses_before_clearing_an_unplaceable_node() {
    let (storage, _dir) = setup().await;
    let locks: raisin_locks::LockManagerHandle =
        Arc::new(raisin_locks::InProcessLockManager::new());
    let (job, context) = job();
    handler(&storage, &locks, "local")
        .handle(&job, &context)
        .await
        .unwrap();
    assert_eq!(state(&storage), Some(CompoundBuildPhase::Ready));

    let db = storage.db();
    let cf_path = cf_handle(db, cf::NODE_PATH).unwrap();
    let prefix = keys::node_path_key_prefix(T, R, B, WS, "n1");
    let doomed: Vec<Box<[u8]>> = crate::prefix_scan(db, cf_path, &prefix)
        .map(|item| item.unwrap().0)
        .take_while(|key| key.starts_with(&prefix))
        .collect();
    assert!(!doomed.is_empty());
    for key in doomed {
        db.delete_cf(cf_path, key).unwrap();
    }
    crate::compound_state::CompoundStateStore::new(db.clone())
        .mark_not_built(T, R, B, WS, INDEX)
        .unwrap();

    assert!(handler(&storage, &locks, "local")
        .handle(&job, &context)
        .await
        .is_err());
    assert_eq!(state(&storage), Some(CompoundBuildPhase::NotBuilt));
    let listed = storage
        .compound_index()
        .scan_compound_index(
            StorageScope::new(T, R, B, WS),
            INDEX,
            &[raisin_storage::CompoundColumnValue::String("a".into())],
            false,
            true,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(listed.len(), 3, "the refused build must not clear entries");
}
