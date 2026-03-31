//! Detailed profiling of the put() operation to identify bottlenecks
//!
//! Run with: `cargo test --package raisin-rocksdb --test profile_put_operation -- --nocapture`
//!
//! This test profiles each step of the put() operation to understand where time is spent:
//! - Revision allocation
//! - Order label calculation
//! - Index updates
//! - RocksDB write operations

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
        description: Some("Put operation profiling test".to_string()),
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

struct PutTiming {
    node_number: usize,
    total_time_us: u128,
}

#[tokio::test]
async fn profile_put_operation() -> Result<()> {
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║         Detailed Put Operation Profiling                ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    let (storage, _temp_dir) = setup_storage().await?;
    let nodes_repo = storage.nodes();

    // Profile different batch sizes to see the degradation pattern
    let test_sizes = vec![10, 50, 100, 200, 500];

    for &size in &test_sizes {
        println!("\n┌──────────────────────────────────────────────────────────┐");
        println!(
            "│ Profiling {} nodes                                     ",
            size
        );
        println!("└──────────────────────────────────────────────────────────┘\n");

        let mut timings = Vec::new();
        let overall_start = Instant::now();

        // Create nodes and time each put operation
        for i in 0..size {
            let node = create_node(i);

            let start = Instant::now();
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
            let elapsed = start.elapsed().as_micros();

            timings.push(PutTiming {
                node_number: i,
                total_time_us: elapsed,
            });

            // Print detailed timing for select nodes to show pattern
            if i < 5 || i == 9 || i == 49 || i == 99 || i == 199 || i == 499 || i == size - 1 {
                println!(
                    "  Node {:4}: {:8} μs ({:6.2} ms)",
                    i,
                    elapsed,
                    elapsed as f64 / 1000.0
                );
            }
        }

        let overall_elapsed = overall_start.elapsed();

        // Calculate statistics
        let total_time: u128 = timings.iter().map(|t| t.total_time_us).sum();
        let avg_time = total_time / timings.len() as u128;
        let min_time = timings.iter().map(|t| t.total_time_us).min().unwrap();
        let max_time = timings.iter().map(|t| t.total_time_us).max().unwrap();

        // Calculate first 10 vs last 10 to show degradation
        let first_10_avg: u128 = timings
            .iter()
            .take(10)
            .map(|t| t.total_time_us)
            .sum::<u128>()
            / 10;
        let last_10_avg: u128 = timings
            .iter()
            .rev()
            .take(10)
            .map(|t| t.total_time_us)
            .sum::<u128>()
            / 10;

        println!("\n  ┌─────────────────────────────────────────────────────┐");
        println!(
            "  │ Statistics for {} nodes                          │",
            size
        );
        println!("  ├─────────────────────────────────────────────────────┤");
        println!(
            "  │ Total time:        {:8.2} ms                    │",
            overall_elapsed.as_millis()
        );
        println!(
            "  │ Throughput:        {:8.0} ops/sec                │",
            size as f64 / overall_elapsed.as_secs_f64()
        );
        println!(
            "  │ Average:           {:8} μs ({:6.2} ms)         │",
            avg_time,
            avg_time as f64 / 1000.0
        );
        println!(
            "  │ Min:               {:8} μs ({:6.2} ms)         │",
            min_time,
            min_time as f64 / 1000.0
        );
        println!(
            "  │ Max:               {:8} μs ({:6.2} ms)         │",
            max_time,
            max_time as f64 / 1000.0
        );
        println!("  ├─────────────────────────────────────────────────────┤");
        println!(
            "  │ First 10 avg:      {:8} μs ({:6.2} ms)         │",
            first_10_avg,
            first_10_avg as f64 / 1000.0
        );
        println!(
            "  │ Last 10 avg:       {:8} μs ({:6.2} ms)         │",
            last_10_avg,
            last_10_avg as f64 / 1000.0
        );
        println!(
            "  │ Slowdown factor:   {:8.2}x                      │",
            last_10_avg as f64 / first_10_avg as f64
        );
        println!("  └─────────────────────────────────────────────────────┘");

        // Show percentile distribution for larger batches
        if size >= 100 {
            let mut sorted_times: Vec<u128> = timings.iter().map(|t| t.total_time_us).collect();
            sorted_times.sort();

            let p50 = sorted_times[size * 50 / 100];
            let p90 = sorted_times[size * 90 / 100];
            let p95 = sorted_times[size * 95 / 100];
            let p99 = sorted_times[size * 99 / 100];

            println!("\n  Percentiles:");
            println!("    P50: {:8} μs ({:6.2} ms)", p50, p50 as f64 / 1000.0);
            println!("    P90: {:8} μs ({:6.2} ms)", p90, p90 as f64 / 1000.0);
            println!("    P95: {:8} μs ({:6.2} ms)", p95, p95 as f64 / 1000.0);
            println!("    P99: {:8} μs ({:6.2} ms)", p99, p99 as f64 / 1000.0);
        }

        // Analyze time growth pattern
        if size >= 50 {
            println!("\n  Time growth pattern:");
            let buckets = [(0, 10), (10, 20), (20, 30), (40, 50)];
            for (start, end) in buckets {
                if end <= size {
                    let bucket_avg: u128 = timings[start..end]
                        .iter()
                        .map(|t| t.total_time_us)
                        .sum::<u128>()
                        / (end - start) as u128;
                    println!(
                        "    Nodes {:3}-{:3}: {:8} μs ({:6.2} ms)",
                        start,
                        end - 1,
                        bucket_avg,
                        bucket_avg as f64 / 1000.0
                    );
                }
            }

            if size >= 200 {
                let buckets = [(100, 110), (150, 160), (190, 200)];
                for (start, end) in buckets {
                    if end <= size {
                        let bucket_avg: u128 = timings[start..end]
                            .iter()
                            .map(|t| t.total_time_us)
                            .sum::<u128>()
                            / (end - start) as u128;
                        println!(
                            "    Nodes {:3}-{:3}: {:8} μs ({:6.2} ms)",
                            start,
                            end - 1,
                            bucket_avg,
                            bucket_avg as f64 / 1000.0
                        );
                    }
                }
            }
        }
    }

    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║                    Analysis Summary                      ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("\nIf you see significant slowdown (>2x) from first to last 10:");
    println!("  • Likely O(n) or O(n²) operation in the put path");
    println!("  • Check: get_last_order_label, get_order_label_for_child");
    println!("  • Check: RocksDB LSM tree compaction overhead");
    println!("\nIf slowdown is consistent across all sizes:");
    println!("  • Likely constant overhead (serialization, batching)");
    println!("  • Normal for small batches\n");

    Ok(())
}
