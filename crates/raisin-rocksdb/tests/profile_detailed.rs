//! Detailed profiling with timing for each operation
//! Run with: RUST_LOG=info cargo test --release --package raisin-rocksdb --test profile_detailed -- --nocapture --test-threads=1

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

struct TimingStats {
    count: usize,
    total: Duration,
    min: Duration,
    max: Duration,
}

impl TimingStats {
    fn new() -> Self {
        Self {
            count: 0,
            total: Duration::ZERO,
            min: Duration::from_secs(999999),
            max: Duration::ZERO,
        }
    }

    fn record(&mut self, duration: Duration) {
        self.count += 1;
        self.total += duration;
        if duration < self.min {
            self.min = duration;
        }
        if duration > self.max {
            self.max = duration;
        }
    }

    fn avg_micros(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total.as_micros() as f64 / self.count as f64
        }
    }
}

#[tokio::test]
async fn profile_node_operations() -> Result<()> {
    println!("\n🔬 DETAILED PROFILING: Node creation timing breakdown");
    println!("======================================================\n");

    // Setup
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(temp_dir.path())?;

    let registry = storage.registry();
    registry.register_tenant(TENANT, HashMap::new()).await?;

    let repo_mgmt = storage.repository_management();
    let repo_config = RepositoryConfig {
        default_branch: BRANCH.to_string(),
        description: Some("Profile test".to_string()),
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

    // Profile different batch sizes
    for node_count in [10, 50, 100, 500] {
        println!("📊 Creating {} nodes...", node_count);

        let mut stats = TimingStats::new();
        let start = Instant::now();

        for i in 0..node_count {
            let node_start = Instant::now();

            let node = Node {
                id: uuid::Uuid::new_v4().to_string(),
                path: format!("/profile{:05}", i),
                name: format!("profile{:05}", i),
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
                    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                    node,
                    CreateNodeOptions::default(),
                )
                .await?;

            stats.record(node_start.elapsed());
        }

        let elapsed = start.elapsed();
        let rate = node_count as f64 / elapsed.as_secs_f64();

        println!("  Total time: {:.2}s", elapsed.as_secs_f64());
        println!("  Throughput: {:.1} nodes/sec", rate);
        println!(
            "  Per node:   {:.2}ms avg, {:.2}ms min, {:.2}ms max\n",
            stats.avg_micros() / 1000.0,
            stats.min.as_micros() as f64 / 1000.0,
            stats.max.as_micros() as f64 / 1000.0
        );
    }

    Ok(())
}
