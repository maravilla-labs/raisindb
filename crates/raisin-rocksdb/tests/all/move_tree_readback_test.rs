//! A node moved inside a transaction must be readable at its NEW path for the
//! rest of that transaction.
//!
//! `move_node_tree` wrote the new `PATH_INDEX` entry into the batch but only
//! marked the OLD path as deleted in the transaction read cache. In-transaction
//! reads resolve through that cache, so the moved node was invisible at its new
//! path until commit. A caller that moved a node and then upserted it there
//! took the CREATE branch and produced a duplicate — which, for a node type
//! carrying a `unique: true` property, failed the write outright.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{fractional_index, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeRepository, RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;

const TENANT: &str = "movereadback-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "default";

async fn setup() -> (Arc<RocksDBStorage>, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());

    storage
        .repository_management()
        .create_repository(TENANT, REPO, RepositoryConfig::default())
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await
        .unwrap();

    let folder = NodeType {
        id: Some("raisin:Folder".to_string()),
        strict: Some(false),
        name: "raisin:Folder".to_string(),
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
        versionable: Some(true),
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
            folder,
            CommitMetadata::system("seed folder type"),
        )
        .await
        .unwrap();

    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await
        .unwrap();

    (storage, dir)
}

fn node(path: &str) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.clone()));
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_moved_node_resolves_at_its_new_path_within_the_same_transaction() {
    let (storage, _dir) = setup().await;

    // Seed /old in its own committed transaction.
    let seeded = {
        let tx = storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch(BRANCH).unwrap();
        tx.set_actor("test").unwrap();
        tx.set_message("seed").unwrap();
        tx.set_auth_context(AuthContext::system()).unwrap();
        let n = node("/old");
        let id = n.id.clone();
        tx.upsert_deep_node(WS, &n, "raisin:Folder").await.unwrap();
        tx.commit().await.unwrap();
        id
    };

    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_message("move").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();

    tx.move_node_tree(WS, &seeded, "/new").await.unwrap();

    // The whole point: read-your-writes must cover the arrival, not just the
    // departure.
    let at_new = tx.get_node_by_path(WS, "/new").await.unwrap();
    assert_eq!(
        at_new.map(|n| n.id),
        Some(seeded.clone()),
        "the moved node must be readable at its new path before commit"
    );
    assert!(
        tx.get_node_by_path(WS, "/old").await.unwrap().is_none(),
        "the vacated path must not still resolve"
    );

    // And an upsert at the new path must UPDATE it, not create a twin.
    let mut moved = node("/new");
    moved.properties.insert(
        "title".to_string(),
        PropertyValue::String("renamed".to_string()),
    );
    tx.upsert_deep_node(WS, &moved, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let after = tx_read(&storage, "/new").await.expect("node at /new");
    assert_eq!(
        after.id, seeded,
        "upserting at the new path must reuse the moved node's id"
    );
    assert_eq!(
        after.properties.get("title"),
        Some(&PropertyValue::String("renamed".to_string()))
    );
    assert!(tx_read(&storage, "/old").await.is_none());
}

async fn tx_read(storage: &Arc<RocksDBStorage>, path: &str) -> Option<Node> {
    use raisin_storage::{scope::StorageScope, NodeRepository};
    storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
        .await
        .unwrap()
}
