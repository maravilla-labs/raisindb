//! The replication apply path's baseline reader (`load_latest_node`).
//!
//! A replicated upsert diffs the incoming node against the stored one to
//! tombstone what it supersedes. The baseline used to come from a scan of the
//! WHOLE branch's NODES column family, decoded as a full `Node` — so a
//! repository-written `StorageNode` blob came back with `path == ""` and a
//! replicated rename never tombstoned the old path. These pin the one-seek
//! read, the path materialization, and that only an id-only read scans.

use super::perf_counters::count;
use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::replication::{OperationApplicator, WorkspaceHint};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "rb-tenant";
const REPO: &str = "rb-repo";
const BRANCH: &str = "main";
const WS: &str = "content";

async fn setup() -> (Arc<RocksDBStorage>, OperationApplicator, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).expect("storage");
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await
        .expect("tenant");
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await
        .expect("repo");
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await
        .expect("branch");
    for ws in [WS, "default"] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .expect("workspace");
    }
    let storage = Arc::new(storage);
    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );
    (storage, applicator, temp_dir)
}

fn folder(name: &str) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: format!("/{name}"),
        name: name.to_string(),
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

/// Create through the repository, which writes a `StorageNode` blob (no
/// path) plus the NODE_PATH entry.
async fn create(storage: &RocksDBStorage, name: &str) -> Node {
    let node = folder(name);
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, WS),
            node.clone(),
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await
        .expect("create");
    node
}

#[tokio::test]
async fn load_latest_node_materializes_path_for_storage_node_blob() {
    let (storage, applicator, _dir) = setup().await;
    let node = create(&storage, "folder-a").await;

    let loaded = applicator
        .load_latest_node(TENANT, REPO, BRANCH, WorkspaceHint::Explicit(WS), &node.id)
        .expect("load")
        .expect("found");
    assert_eq!(loaded.path, "/folder-a");
    assert_eq!(loaded.id, node.id);

    // The bounded variant: nothing exists strictly before the first revision.
    let head = storage
        .branches()
        .get_head(TENANT, REPO, BRANCH)
        .await
        .unwrap();
    let before = applicator
        .load_node_before(
            TENANT,
            REPO,
            BRANCH,
            WorkspaceHint::Explicit(WS),
            &node.id,
            &head,
        )
        .expect("load before");
    assert!(before.is_none(), "{before:?}");
    let at_next = HLC::new(head.timestamp_ms, head.counter + 1);
    let before_next = applicator
        .load_node_before(
            TENANT,
            REPO,
            BRANCH,
            WorkspaceHint::Explicit(WS),
            &node.id,
            &at_next,
        )
        .expect("load before next")
        .expect("the version at head is strictly below head+1");
    assert_eq!(before_next.path, "/folder-a");
}

#[tokio::test]
async fn replicated_upsert_finds_previous_version_without_branch_scan() {
    let (storage, applicator, _dir) = setup().await;
    for i in 0..300 {
        create(&storage, &format!("distractor-{i}")).await;
    }
    let target = create(&storage, "old-name").await;

    // The scoped read is one seek, however big the branch is...
    let (scoped, scoped_counts) = count(|| {
        applicator.load_latest_node(
            TENANT,
            REPO,
            BRANCH,
            WorkspaceHint::Explicit(WS),
            &target.id,
        )
    });
    assert!(scoped.expect("load").is_some());
    // ...where the branch scan steps over the other nodes' keys.
    let (scanned, scan_counts) = count(|| {
        applicator.load_latest_node(TENANT, REPO, BRANCH, WorkspaceHint::Unknown, &target.id)
    });
    assert!(scanned.expect("load").is_some());
    assert!(
        scoped_counts.next_on_memtable < 20,
        "scoped baseline read stepped {} keys",
        scoped_counts.next_on_memtable
    );
    assert!(
        scan_counts.next_on_memtable > scoped_counts.next_on_memtable,
        "the counter must tell a scan from a seek: scan {:?} vs seek {:?}",
        scan_counts,
        scoped_counts
    );

    // And the baseline carries the stored PATH, so a replicated rename
    // tombstones the old path instead of leaving it live.
    let mut renamed = target.clone();
    renamed.name = "new-name".to_string();
    renamed.path = "/new-name".to_string();
    renamed.workspace = Some(WS.to_string());
    renamed.order_key = "a0".to_string();
    let revision = HLC::new(chrono::Utc::now().timestamp_millis() as u64 + 60_000, 0);
    let op = Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision,
            node_changes: vec![ReplicatedNodeChange {
                node: renamed.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: format!("a0::{}", renamed.id),
            }],
        },
        revision: Some(revision),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        agent: None,
        acknowledged_by: Default::default(),
    };
    applicator.apply_operation(&op).await.expect("apply");

    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let by_new = storage
        .nodes()
        .get_by_path(scope, "/new-name", None)
        .await
        .unwrap();
    assert_eq!(by_new.map(|n| n.id), Some(target.id.clone()));
    let by_old = storage
        .nodes()
        .get_by_path(scope, "/old-name", None)
        .await
        .unwrap();
    assert!(
        by_old.is_none(),
        "the replicated rename left the old path live"
    );
}

/// A workspace the op names is the authority: a node living elsewhere is a
/// plain miss there — no branch scan (the defaulted-workspace fallback for
/// pre-v2 peers is gone, plan "Phase 11d"); only an id-only read scans.
#[tokio::test]
async fn an_explicit_workspace_miss_does_not_scan() {
    let (storage, applicator, _dir) = setup().await;
    let node = create(&storage, "elsewhere").await;

    let explicit = applicator
        .load_latest_node(
            TENANT,
            REPO,
            BRANCH,
            WorkspaceHint::Explicit("default"),
            &node.id,
        )
        .expect("load");
    assert!(explicit.is_none(), "a named miss is a miss, never a scan");

    let found = applicator
        .load_latest_node(TENANT, REPO, BRANCH, WorkspaceHint::Unknown, &node.id)
        .expect("load")
        .expect("an id-only read finds it by scan");
    assert_eq!(found.path, "/elsewhere");
}
