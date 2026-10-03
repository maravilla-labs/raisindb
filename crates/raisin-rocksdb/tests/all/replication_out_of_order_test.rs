//! Replicated upserts that arrive OUT OF ORDER, and the id-only baseline read.
//!
//! Replication does not deliver in revision order. When r2 is applied before
//! r1, r1's index entries must still be right for a time-travel read at r1 and
//! must NOT stay live at HEAD: nothing newer than r2 will ever supersede them.

use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::replication::{unknown_workspace_scans, OperationApplicator};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository,
    PropertyIndexRepository, ReferenceIndexRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "oo-tenant";
const REPO: &str = "oo-repo";
const BRANCH: &str = "main";
const WS: &str = "content";
const OTHER_WS: &str = "archive"; // before `content`: a branch scan meets it first

fn scope(ws: &str) -> StorageScope<'_> {
    StorageScope::new(TENANT, REPO, BRANCH, ws)
}

async fn setup() -> (Arc<RocksDBStorage>, OperationApplicator, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(dir.path()).expect("storage");
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await
        .unwrap();
    let config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: None,
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, config)
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await
        .unwrap();
    for ws in [WS, OTHER_WS, "default"] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .unwrap();
    }
    let storage = Arc::new(storage);
    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );
    (storage, applicator, dir)
}

async fn create(storage: &RocksDBStorage, ws: &str, node: &Node) {
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    storage
        .nodes()
        .create(scope(ws), node.clone(), options)
        .await
        .expect("create");
}

fn folder(name: &str) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: format!("/{name}"),
        name: name.to_string(),
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        workspace: Some(WS.to_string()),
        ..Node::default()
    }
}

/// `base` as it stands at `revision`: renamed to `name`, titled `title`.
fn version(base: &Node, name: &str, title: &str, refers_to: Option<&Node>) -> Node {
    let mut node = base.clone();
    node.name = name.to_string();
    node.path = format!("/{name}");
    node.workspace = Some(WS.to_string());
    node.properties.clear();
    node.properties
        .insert("title".into(), PropertyValue::String(title.into()));
    if let Some(target) = refers_to {
        node.properties.insert(
            "related".into(),
            PropertyValue::Reference(RaisinReference {
                id: target.id.clone(),
                workspace: WS.to_string(),
                path: target.path.clone(),
            }),
        );
    }
    node
}

fn later(offset_ms: u64) -> HLC {
    HLC::new(chrono::Utc::now().timestamp_millis() as u64 + offset_ms, 0)
}

fn op(revision: HLC, op_type: OpType) -> Operation {
    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type,
        revision: Some(revision),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        agent: None,
        acknowledged_by: Default::default(),
    }
}

async fn upsert(applicator: &OperationApplicator, node: &Node, label: &str, revision: HLC) {
    let change = ReplicatedNodeChange {
        node: node.clone(),
        parent_id: Some("/".to_string()),
        kind: ReplicatedNodeChangeKind::Upsert,
        cf_order_key: format!("{label}::{}", node.id),
    };
    let op_type = OpType::ApplyRevision {
        branch_head: revision,
        node_changes: vec![change],
    };
    applicator
        .apply_operation(&op(revision, op_type))
        .await
        .expect("apply upsert");
}

async fn title_count(storage: &RocksDBStorage, title: &str, at: Option<&HLC>) -> usize {
    let value = PropertyValue::String(title.to_string());
    storage
        .property_index()
        .count_by_property(scope(WS), "title", &value, false, at)
        .await
        .expect("count")
}

async fn path_resolves(storage: &RocksDBStorage, path: &str, at: Option<&HLC>) -> bool {
    let found = storage.nodes().get_by_path(scope(WS), path, at).await;
    found.expect("get_by_path").is_some()
}

#[tokio::test]
async fn replicated_op_older_than_head_does_not_leave_live_entries() {
    let (storage, applicator, _dir) = setup().await;
    let target = folder("target");
    create(&storage, WS, &target).await;
    let doc = folder("doc");
    create(&storage, WS, &version(&doc, "doc", "A", None)).await;

    // r2 arrives first, then r1 (r0 < r1 < r2).
    let (r1, r2) = (later(60_000), later(120_000));
    upsert(&applicator, &version(&doc, "doc-c", "C", None), "a2", r2).await;
    upsert(
        &applicator,
        &version(&doc, "doc-b", "B", Some(&target)),
        "a1",
        r1,
    )
    .await;

    // HEAD is r2: nothing of r1 matches.
    assert_eq!(title_count(&storage, "C", None).await, 1);
    assert_eq!(
        title_count(&storage, "B", None).await,
        0,
        "r1's title is live at HEAD"
    );
    assert!(path_resolves(&storage, "/doc-c", None).await);
    assert!(
        !path_resolves(&storage, "/doc-b", None).await,
        "r1's path is live at HEAD"
    );
    assert!(!path_resolves(&storage, "/doc", None).await);
    let referrers = storage
        .reference_index()
        .find_referencing_nodes(scope(WS), WS, &target.id, false)
        .await
        .unwrap();
    assert!(
        referrers.is_empty(),
        "r1's reference is live at HEAD: {referrers:?}"
    );

    // Time travel to r1 sees r1, written in full.
    assert_eq!(title_count(&storage, "B", Some(&r1)).await, 1);
    assert_eq!(title_count(&storage, "A", Some(&r1)).await, 0);
    assert!(path_resolves(&storage, "/doc-b", Some(&r1)).await);
    assert!(!path_resolves(&storage, "/doc", Some(&r1)).await);
    let at_r1 = storage.nodes().get(scope(WS), &doc.id, Some(&r1)).await;
    let at_r1 = at_r1.unwrap().expect("doc at r1");
    assert_eq!(at_r1.path, "/doc-b");
}

/// An id-only delete (no workspace) must find the LIVE copy of the id, not the
/// workspace where the same id was deleted earlier.
#[tokio::test]
async fn id_only_delete_resolves_the_live_workspace() {
    let (storage, applicator, _dir) = setup().await;
    let doc = folder("doc");
    create(&storage, OTHER_WS, &doc).await;
    let options = DeleteNodeOptions::default();
    let deleted = storage
        .nodes()
        .delete(scope(OTHER_WS), &doc.id, options)
        .await;
    assert!(deleted.expect("delete"));
    upsert(
        &applicator,
        &version(&doc, "doc", "live", None),
        "a1",
        later(60_000),
    )
    .await;
    assert!(path_resolves(&storage, "/doc", None).await);

    let scans = unknown_workspace_scans();
    let delete = OpType::DeleteNode { node_id: doc.id };
    applicator
        .apply_operation(&op(later(120_000), delete))
        .await
        .expect("apply delete");
    assert!(
        unknown_workspace_scans() > scans,
        "the id-only read is counted"
    );
    assert!(
        !path_resolves(&storage, "/doc", None).await,
        "the delete went to the already-deleted workspace and left the live node"
    );
}

/// A move, then a replicated upsert of the moved node: the old path is gone.
#[tokio::test]
async fn move_then_replicated_upsert_leaves_old_path_unresolvable() {
    let (storage, applicator, _dir) = setup().await;
    let doc = folder("before");
    create(&storage, WS, &doc).await;
    storage
        .nodes()
        .move_node(scope(WS), &doc.id, "/after", None)
        .await
        .expect("move");
    upsert(
        &applicator,
        &version(&doc, "after", "edited", None),
        "a1",
        later(60_000),
    )
    .await;

    assert!(path_resolves(&storage, "/after", None).await);
    assert!(
        !path_resolves(&storage, "/before", None).await,
        "the old path resolves after a move and a replicated upsert"
    );
}
