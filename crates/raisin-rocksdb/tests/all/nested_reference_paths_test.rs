//! The last two reference traversals that only saw TOP-LEVEL references: the
//! legacy replicated `CreateNode` writer and the cross-branch prune tombstoner.
//! Both now go through the one walker, so a reference inside an array is
//! indexed on a replica and retired when a promotion prunes its node.

use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_replication::{OpType, Operation, VectorClock};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository,
    ReferenceIndexRepository, RegistryRepository, RepoScope, RepositoryManagementRepository,
    Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "nr-tenant";
const REPO: &str = "nr-repo";
const WS: &str = "content";

fn scope(branch: &str) -> StorageScope<'_> {
    StorageScope::new(TENANT, REPO, branch, WS)
}

async fn setup() -> (Arc<RocksDBStorage>, TempDir) {
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
        default_branch: "main".to_string(),
        description: None,
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, config)
        .await
        .unwrap();
    for branch in ["main", "publish"] {
        storage
            .branches()
            .create_branch(TENANT, REPO, branch, "system", None, None, false, false)
            .await
            .unwrap();
    }
    let workspace = raisin_models::workspace::Workspace::new(WS.to_string());
    storage
        .workspaces()
        .put(RepoScope::new(TENANT, REPO), workspace)
        .await
        .unwrap();
    (Arc::new(storage), dir)
}

fn node(name: &str, path: &str) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name: name.to_string(),
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

/// `{ "tags": [ <reference to target> ] }` — reachable only by walking.
fn nested_reference(target: &Node) -> HashMap<String, PropertyValue> {
    let reference = PropertyValue::Reference(RaisinReference {
        id: target.id.clone(),
        workspace: WS.to_string(),
        path: target.path.clone(),
    });
    HashMap::from([("tags".to_string(), PropertyValue::Array(vec![reference]))])
}

async fn create(storage: &RocksDBStorage, node: &Node) {
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    let created = storage.nodes().create(scope("main"), node.clone(), options);
    created.await.expect("create");
}

async fn referrers(storage: &RocksDBStorage, branch: &str, target: &Node) -> Vec<(String, String)> {
    storage
        .reference_index()
        .find_referencing_nodes(scope(branch), WS, &target.id, false)
        .await
        .expect("reference read")
}

#[tokio::test]
async fn legacy_create_indexes_nested_references() {
    let (storage, _dir) = setup().await;
    let target = node("target", "/target");
    create(&storage, &target).await;
    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let source_id = uuid::Uuid::new_v4().to_string();
    let revision = HLC::new(chrono::Utc::now().timestamp_millis() as u64 + 60_000, 0);
    let op_type = OpType::CreateNode {
        node_id: source_id.clone(),
        name: "source".to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        parent_id: None,
        order_key: "a0".to_string(),
        properties: nested_reference(&target),
        owner_id: None,
        workspace: Some(WS.to_string()),
        path: "/source".to_string(),
    };
    let op = Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: "main".to_string(),
        op_type,
        revision: Some(revision),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        agent: None,
        acknowledged_by: Default::default(),
    };
    applicator.apply_operation(&op).await.expect("apply");

    assert_eq!(
        referrers(&storage, "main", &target).await,
        vec![(source_id, "tags.0".to_string())],
        "a replicated create must index a nested reference at its dot path"
    );
}

#[tokio::test]
async fn prune_tombstones_nested_references() {
    let (storage, _dir) = setup().await;
    let target = node("target", "/target");
    create(&storage, &target).await;
    let section = node("section", "/section");
    create(&storage, &section).await;
    let mut page = node("page", "/section/page");
    page.parent = Some("section".to_string());
    page.properties = nested_reference(&target);
    create(&storage, &page).await;

    let roots = vec!["/target".to_string(), "/section".to_string()];
    let promote = || {
        storage.nodes().copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WS, &roots, true, true, None, None,
        )
    };
    promote().await.expect("first promotion");
    assert_eq!(referrers(&storage, "publish", &target).await.len(), 1);

    // The page goes on main; the next promotion prunes it from publish.
    let options = DeleteNodeOptions::default();
    let deleted = storage
        .nodes()
        .delete(scope("main"), &page.id, options)
        .await;
    assert!(deleted.expect("delete"));
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    promote().await.expect("second promotion");

    let left = referrers(&storage, "publish", &target).await;
    assert!(
        left.is_empty(),
        "a pruned node's nested reference is still live on publish: {left:?}"
    );
}
