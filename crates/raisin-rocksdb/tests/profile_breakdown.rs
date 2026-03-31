//! Deep profiling - measure each internal operation
//! Run with: cargo test --release --package raisin-rocksdb --test profile_breakdown -- --nocapture

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

#[tokio::test]
async fn profile_operation_breakdown() -> Result<()> {
    println!("\n🔬 DEEP PROFILING: Per-operation timing");
    println!("=========================================\n");

    let repo_config = RepositoryConfig {
        default_branch: BRANCH.to_string(),
        description: Some("Profile test".to_string()),
        tags: HashMap::new(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
    };

    // Test at different scales to see degradation
    for count in [100, 200, 500, 1000] {
        // Create fresh storage for each test
        let temp_dir =
            tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
        let storage = RocksDBStorage::new(temp_dir.path())?;

        let registry = storage.registry();
        registry.register_tenant(TENANT, HashMap::new()).await?;

        let repo_mgmt = storage.repository_management();
        repo_mgmt
            .create_repository(TENANT, REPO, repo_config.clone())
            .await?;

        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
            .await?;

        let workspace = Workspace::new(WORKSPACE.to_string());
        let workspace_service = WorkspaceService::new(Arc::new(storage.clone()));
        workspace_service.put(TENANT, REPO, workspace).await?;

        let nodes = storage.nodes();
        println!("📊 Testing with {} nodes total:", count);

        // Measure individual operations at different points
        let test_points = [count / 4, count / 2, count * 3 / 4, count];

        for &test_at in &test_points {
            // Create nodes up to test point
            for i in 0..test_at {
                let node = Node {
                    id: uuid::Uuid::new_v4().to_string(),
                    path: format!("/bench{:05}", i),
                    name: format!("bench{:05}", i),
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

                nodes
                    .create(
                        TENANT,
                        REPO,
                        BRANCH,
                        WORKSPACE,
                        node,
                        CreateNodeOptions::default(),
                    )
                    .await?;
            }

            // Now measure a single insert at this scale
            let start = Instant::now();
            let node = Node {
                id: uuid::Uuid::new_v4().to_string(),
                path: format!("/measure{:05}", test_at),
                name: format!("measure{:05}", test_at),
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
            nodes
                .create(
                    TENANT,
                    REPO,
                    BRANCH,
                    WORKSPACE,
                    node,
                    CreateNodeOptions::default(),
                )
                .await?;
            let insert_time = start.elapsed();

            println!(
                "  At {} nodes: single insert took {:.2}ms",
                test_at,
                insert_time.as_micros() as f64 / 1000.0
            );
        }

        println!();
        // Storage and temp_dir will be automatically dropped here at end of loop iteration
    }

    Ok(())
}
