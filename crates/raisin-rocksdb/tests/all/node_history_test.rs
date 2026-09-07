//! `NodeRepository::get_node_history` lists every stored revision of a node,
//! newest first. Pinned because the HTTP `/api/history` endpoint returned `[]`
//! for nodes with several committed revisions.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::{fractional_index, RocksDBStorage};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use tempfile::TempDir;

const TENANT: &str = "history-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

async fn setup() -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path())?);
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
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
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WORKSPACE.to_string()),
        )
        .await?;
    Ok((storage, temp_dir))
}

fn build_node(title: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );
    Node {
        id: "hist-node".to_string(),
        name: "hist".to_string(),
        path: "/hist".to_string(),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        parent: Some("/".to_string()),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

async fn commit(storage: &Arc<RocksDBStorage>, node: &Node, message: &str) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message(message)?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.put_node(WORKSPACE, node).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn history_lists_every_committed_revision_newest_first() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE);

    commit(&storage, &build_node("v1"), "create").await?;
    commit(&storage, &build_node("v2"), "update 1").await?;
    commit(&storage, &build_node("v3"), "update 2").await?;

    let current = storage.nodes().get(scope, "hist-node", None).await?;
    assert!(current.is_some(), "node must be readable at HEAD");

    let history = storage
        .nodes()
        .get_node_history(scope, "hist-node", None)
        .await?;
    assert_eq!(
        history.len(),
        3,
        "three commits ⇒ three revisions, got {:?}",
        history
    );
    assert!(
        history.windows(2).all(|w| w[0].revision > w[1].revision),
        "newest first"
    );
    assert!(history.iter().all(|e| !e.deleted));

    let limited = storage
        .nodes()
        .get_node_history(scope, "hist-node", Some(2))
        .await?;
    assert_eq!(limited.len(), 2);
    Ok(())
}
