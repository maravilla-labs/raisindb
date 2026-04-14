use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, ListOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

#[tokio::test]
async fn test_100_nodes_sequential_creation() -> Result<()> {
    // Setup
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(temp_dir.path())?;

    let registry = storage.registry();
    registry.register_tenant(TENANT, HashMap::new()).await?;

    let repo_mgmt = storage.repository_management();
    let repo_config = RepositoryConfig {
        default_branch: BRANCH.to_string(),
        description: Some("100-node test".to_string()),
        tags: HashMap::new(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
    };
    repo_mgmt
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await?;

    let workspace = Workspace::new(WORKSPACE.to_string());
    let workspace_service = WorkspaceService::new(Arc::new(storage.clone()));
    workspace_service.put(TENANT, REPO, workspace).await?;

    let nodes = storage.nodes();

    // Create 100 nodes at root level
    println!("Creating 100 nodes...");
    for i in 0..100 {
        let node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            path: format!("/node{:03}", i),
            name: format!("node{:03}", i),
            parent: Some("/".to_string()),
            node_type: "raisin:Page".to_string(),
            properties: HashMap::new(),
            children: Vec::new(),
            order_key: "temp".to_string(),
            has_children: None,
            version: 1,
            archetype: None,
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            created_by: Some("test-user".to_string()),
            updated_by: Some("test-user".to_string()),
            published_at: None,
            published_by: None,
            translations: None,
            tenant_id: Some(TENANT.to_string()),
            workspace: Some(WORKSPACE.to_string()),
            owner_id: None,
            relations: vec![],
        };

        let options = CreateNodeOptions::default();
        nodes
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                node,
                options,
            )
            .await?;

        if (i + 1) % 10 == 0 {
            println!("  Created {} nodes", i + 1);
        }
    }

    println!("\nListing all root nodes...");
    let root_nodes = nodes
        .list_root(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            ListOptions::default(),
        )
        .await?;

    println!("Found {} nodes (expected 100)", root_nodes.len());

    if root_nodes.len() != 100 {
        println!("\nERROR: Expected 100 nodes but found {}", root_nodes.len());
        println!("First 10 nodes:");
        for (i, node) in root_nodes.iter().take(10).enumerate() {
            println!("  [{}] {} (order_key: {})", i, node.name, node.order_key);
        }
        if root_nodes.len() > 10 {
            println!("Last 10 nodes:");
            let start = root_nodes.len().saturating_sub(10);
            for (i, node) in root_nodes.iter().skip(start).enumerate() {
                println!(
                    "  [{}] {} (order_key: {})",
                    start + i,
                    node.name,
                    node.order_key
                );
            }
        }
    } else {
        println!("SUCCESS: All 100 nodes found!");
    }

    assert_eq!(root_nodes.len(), 100, "Should find all 100 nodes");

    Ok(())
}
