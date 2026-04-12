//! Simplified profiling test with inline timing display
//!
//! Run with: `cargo test --package raisin-rocksdb --test profile_put_detailed -- --nocapture`

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
use raisin_storage::scope::StorageScope;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "test-workspace";

/// Setup storage with tenant, repo, branch, and workspace
async fn setup_storage() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(temp_dir.path())?;

    // Initialize tenant
    let registry = storage.registry();
    registry.register_tenant(TENANT, HashMap::new()).await?;

    // Create repository
    let repo_mgmt = storage.repository_management();
    let repo_config = RepositoryConfig {
        default_branch: BRANCH.to_string(),
        description: Some("Detailed put profiling test".to_string()),
        tags: HashMap::new(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
    };
    repo_mgmt
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    // Create branch
    let branches = storage.branches();
    branches
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await?;

    // Create workspace with ROOT node
    let workspace = Workspace::new(WORKSPACE.to_string());
    let workspace_service = WorkspaceService::new(Arc::new(storage.clone()));
    workspace_service.put(TENANT, REPO, workspace).await?;

    Ok((storage, temp_dir))
}

/// Create a test node
fn create_node(id: usize) -> Node {
    let node_id = uuid::Uuid::new_v4().to_string();
    let name = format!("node{:06}", id);
    let path = format!("/{}", name);

    Node {
        id: node_id,
        path,
        name,
        parent: Some("/".to_string()),
        node_type: "raisin:Page".to_string(),
        properties: HashMap::new(),
        children: Vec::new(),
        order_key: "a0".to_string(),
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
    }
}

#[tokio::test]
async fn profile_put_detailed_100_nodes() -> Result<()> {
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║       Detailed Put Profiling: 100 Nodes                 ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    let (storage, _temp_dir) = setup_storage().await?;
    let nodes_repo = storage.nodes();

    println!("Creating 100 nodes with detailed timing...\n");

    // Show detailed timing for select nodes
    let nodes_to_profile = vec![0, 1, 2, 3, 4, 9, 19, 49, 99];

    for i in 0..100 {
        let node = create_node(i);

        let start = std::time::Instant::now();
        nodes_repo
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                node,
                CreateNodeOptions::default(),
            )
            .await?;
        let elapsed = start.elapsed().as_micros();

        if nodes_to_profile.contains(&i) {
            println!(
                "  Node {:3}: {:6} μs ({:5.2} ms)",
                i,
                elapsed,
                elapsed as f64 / 1000.0
            );
        }
    }

    println!("\n✓ All 100 nodes created");

    // Now show timing breakdown for the last 5 nodes by re-running
    println!("\n┌──────────────────────────────────────────────────────────┐");
    println!("│ Detailed Breakdown for Additional Nodes                 │");
    println!("└──────────────────────────────────────────────────────────┘\n");

    for i in 100..105 {
        let node = create_node(i);

        println!("Node {}:", i);

        let total_start = std::time::Instant::now();

        // Since we can't access internal timing directly, we'll just show total
        nodes_repo
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                node,
                CreateNodeOptions::default(),
            )
            .await?;

        let total = total_start.elapsed().as_micros();
        println!("  Total: {} μs ({:.2} ms)\n", total, total as f64 / 1000.0);
    }

    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║                     Summary                              ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("\nTo see internal timing breakdown:");
    println!("  RUST_LOG=raisin_rocksdb=debug cargo test ... -- --nocapture");
    println!("\nLook for:");
    println!("  PUT_TIMING  - Top-level breakdown of put() operation");
    println!("  ORDER_TIMING - Detailed breakdown of order label calculation\n");

    Ok(())
}
