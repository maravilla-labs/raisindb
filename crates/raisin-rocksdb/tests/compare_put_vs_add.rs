//! Compare put() vs add() performance to prove the bottleneck
//!
//! Run with: `RAISIN_PROFILE=1 cargo test --package raisin-rocksdb --test compare_put_vs_add -- --nocapture`

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
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "test-workspace";

async fn setup_storage() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(temp_dir.path())?;

    let registry = storage.registry();
    registry.register_tenant(TENANT, HashMap::new()).await?;

    let repo_mgmt = storage.repository_management();
    let repo_config = RepositoryConfig {
        default_branch: BRANCH.to_string(),
        description: Some("Performance comparison test".to_string()),
        tags: HashMap::new(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
    };
    repo_mgmt
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    let branches = storage.branches();
    branches
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await?;

    let workspace = Workspace::new(WORKSPACE.to_string());
    let workspace_service = WorkspaceService::new(Arc::new(storage.clone()));
    workspace_service.put(TENANT, REPO, workspace).await?;

    Ok((storage, temp_dir))
}

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
async fn compare_put_vs_add_performance() -> Result<()> {
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║           put() vs add() Performance Comparison         ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    let test_size = 100;

    // Test 1: Using put() (with existence check)
    println!("┌──────────────────────────────────────────────────────────┐");
    println!("│ Test 1: Using put() - with existence check              │");
    println!("└──────────────────────────────────────────────────────────┘\n");

    let (storage, _temp_dir) = setup_storage().await?;
    let nodes_repo = storage.nodes();

    let start = Instant::now();
    for i in 0..test_size {
        let node = create_node(i);
        nodes_repo
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
    let put_duration = start.elapsed();

    println!("  Created {} nodes using put()", test_size);
    println!("  Total time: {:?}", put_duration);
    println!(
        "  Throughput: {:.0} ops/sec",
        test_size as f64 / put_duration.as_secs_f64()
    );
    println!(
        "  Average:    {:.2} ms/node\n",
        put_duration.as_millis() as f64 / test_size as f64
    );

    drop(storage);
    drop(_temp_dir);

    // Test 2: Using add() (NO existence check)
    println!("┌──────────────────────────────────────────────────────────┐");
    println!("│ Test 2: Using add() - NO existence check (optimized)    │");
    println!("└──────────────────────────────────────────────────────────┘\n");

    let (storage, _temp_dir) = setup_storage().await?;
    let nodes_impl = storage.nodes_impl(); // Get the implementation to access add()

    let start = Instant::now();
    for i in 0..test_size {
        let node = create_node(i);
        nodes_impl
            .add(TENANT, REPO, BRANCH, WORKSPACE, node)
            .await?;
    }
    let add_duration = start.elapsed();

    println!("  Created {} nodes using add()", test_size);
    println!("  Total time: {:?}", add_duration);
    println!(
        "  Throughput: {:.0} ops/sec",
        test_size as f64 / add_duration.as_secs_f64()
    );
    println!(
        "  Average:    {:.2} ms/node\n",
        add_duration.as_millis() as f64 / test_size as f64
    );

    // Summary
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                        Results                           ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    let speedup = put_duration.as_secs_f64() / add_duration.as_secs_f64();
    let put_avg_us = put_duration.as_micros() / test_size as u128;
    let add_avg_us = add_duration.as_micros() / test_size as u128;
    let time_saved_us = put_avg_us - add_avg_us;

    println!("  put() average: {} μs/node", put_avg_us);
    println!("  add() average: {} μs/node", add_avg_us);
    println!(
        "  Time saved:    {} μs/node ({:.1}% faster)",
        time_saved_us,
        ((speedup - 1.0) * 100.0)
    );
    println!("  Speedup:       {:.2}x\n", speedup);

    if speedup > 1.5 {
        println!("  ✓ Significant speedup! The existence check is the bottleneck.");
        println!("    The get_order_label_for_child() O(n) scan is causing the slowdown.\n");
    } else {
        println!("  ⚠ Modest speedup. Other factors may be contributing to slowdown.\n");
    }

    Ok(())
}
