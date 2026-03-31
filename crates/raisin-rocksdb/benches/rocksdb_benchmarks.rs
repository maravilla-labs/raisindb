//! Comprehensive benchmarks for RocksDB storage backend.
//!
//! Measures throughput (operations/second) for various scenarios:
//! - Flat structures (nodes in root)
//! - Balanced binary trees
//! - Operations: create, reorder, delete, branch creation, listing
//!
//! ## Quick Test Run
//!
//! Run with: `cargo bench --package raisin-rocksdb`
//!
//! This uses small node counts (20, 50, 100) for quick verification.
//!
//! ## Full Performance Run
//!
//! For comprehensive performance testing with larger datasets (100, 500, 1000, 5000):
//! 1. Change BENCH_SIZES below to FULL_BENCH_SIZES
//! 2. Change sample_size(10) to sample_size(100) in criterion configuration
//! 3. Run: `cargo bench --package raisin-rocksdb -- --sample-size 100`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// Quick test sizes for development/CI
const BENCH_SIZES: &[usize] = &[20, 50, 100];

// Full benchmark sizes for performance analysis (uncomment to use)
// const FULL_BENCH_SIZES: &[usize] = &[100, 500, 1000, 5000];
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, NodeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Test Constants
// ============================================================================

mod constants {
    pub const TENANT: &str = "bench-tenant";
    pub const REPO: &str = "bench-repo";
    pub const BRANCH: &str = "main";
    pub const WORKSPACE: &str = "bench-workspace";
}

// ============================================================================
// Helper Structures and Functions
// ============================================================================

/// Test fixture with isolated RocksDB storage
struct BenchStorage {
    storage: RocksDBStorage,
    _temp_dir: TempDir,
}

impl BenchStorage {
    /// Create a new benchmark storage instance
    async fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let storage = RocksDBStorage::new(temp_dir.path()).expect("Failed to create storage");

        // Initialize tenant, repository, branch, and workspace
        let registry = storage.registry();
        registry
            .register_tenant(constants::TENANT, HashMap::new())
            .await
            .expect("Failed to register tenant");

        let repo_mgmt = storage.repository_management();
        let repo_config = RepositoryConfig {
            default_branch: constants::BRANCH.to_string(),
            description: Some("Benchmark repository".to_string()),
            tags: HashMap::new(),
        };
        repo_mgmt
            .create_repository(constants::TENANT, constants::REPO, repo_config)
            .await
            .expect("Failed to create repository");

        let branches = storage.branches();
        branches
            .create_branch(
                constants::TENANT,
                constants::REPO,
                constants::BRANCH,
                "bench-user",
                None,
                false,
            )
            .await
            .expect("Failed to create branch");

        let workspace = Workspace::new(constants::WORKSPACE.to_string());
        let workspace_service = WorkspaceService::new(Arc::new(storage.clone()));
        workspace_service
            .put(constants::TENANT, constants::REPO, workspace)
            .await
            .expect("Failed to create workspace");

        Self {
            storage,
            _temp_dir: temp_dir,
        }
    }

    fn storage(&self) -> &RocksDBStorage {
        &self.storage
    }

    /// Create a test node
    fn create_node(&self, path: &str, node_type: &str) -> Node {
        let node_id = uuid::Uuid::new_v4().to_string();
        let parts: Vec<&str> = path.rsplitn(2, '/').collect();
        let name = parts[0].to_string();
        let parent = if parts.len() > 1 && !parts[1].is_empty() {
            Some(parts[1].to_string())
        } else {
            None
        };

        Node {
            id: node_id,
            path: path.to_string(),
            name,
            parent,
            node_type: node_type.to_string(),
            properties: HashMap::new(),
            children: Vec::new(),
            order_key: "a0".to_string(),
            has_children: None,
            version: 1,
            archetype: None,
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            created_by: Some("bench-user".to_string()),
            updated_by: Some("bench-user".to_string()),
            published_at: None,
            published_by: None,
            translations: None,
            tenant_id: Some(constants::TENANT.to_string()),
            workspace: Some(constants::WORKSPACE.to_string()),
            owner_id: None,
        }
    }
}

/// Create a flat structure with N nodes at root level
async fn create_flat_structure(storage: &RocksDBStorage, count: usize) -> Vec<String> {
    let bench_storage = BenchStorage {
        storage: storage.clone(),
        _temp_dir: tempfile::tempdir().expect("Failed to create temp dir"),
    };

    let nodes_repo = storage.nodes();
    let mut node_ids = Vec::with_capacity(count);

    for i in 0..count {
        let node = bench_storage.create_node(&format!("/node{:06}", i), "raisin:Page");
        let node_id = node.id.clone();

        nodes_repo
            .put(
                constants::TENANT,
                constants::REPO,
                constants::BRANCH,
                constants::WORKSPACE,
                node,
            )
            .await
            .expect("Failed to create node");

        node_ids.push(node_id);
    }

    node_ids
}

/// Create a balanced binary tree with N nodes
/// Returns a vector of (node_id, path, depth) tuples
async fn create_binary_tree(
    storage: &RocksDBStorage,
    count: usize,
) -> Vec<(String, String, usize)> {
    let bench_storage = BenchStorage {
        storage: storage.clone(),
        _temp_dir: tempfile::tempdir().expect("Failed to create temp dir"),
    };

    let nodes_repo = storage.nodes();
    let mut nodes_info = Vec::with_capacity(count);
    let mut queue: Vec<(String, String, usize)> = Vec::new(); // (path, node_id, depth)

    // Create root of the tree
    let root = bench_storage.create_node("/tree-root", "raisin:Folder");
    let root_path = root.path.clone();
    let root_id = root.id.clone();

    nodes_repo
        .put(
            constants::TENANT,
            constants::REPO,
            constants::BRANCH,
            constants::WORKSPACE,
            root,
        )
        .await
        .expect("Failed to create root");

    nodes_info.push((root_id.clone(), root_path.clone(), 0));
    queue.push((root_path, root_id, 0));

    let mut created = 1;
    let mut node_counter = 0;

    // Build binary tree level by level
    while created < count && !queue.is_empty() {
        let (parent_path, _parent_id, depth) = queue.remove(0);

        // Create left child
        if created < count {
            let left_path = format!("{}/l{:06}", parent_path, node_counter);
            node_counter += 1;

            let left_node = bench_storage.create_node(&left_path, "raisin:Folder");
            let left_id = left_node.id.clone();

            nodes_repo
                .put(
                    constants::TENANT,
                    constants::REPO,
                    constants::BRANCH,
                    constants::WORKSPACE,
                    left_node,
                )
                .await
                .expect("Failed to create left child");

            nodes_info.push((left_id.clone(), left_path.clone(), depth + 1));
            queue.push((left_path, left_id, depth + 1));
            created += 1;
        }

        // Create right child
        if created < count {
            let right_path = format!("{}/r{:06}", parent_path, node_counter);
            node_counter += 1;

            let right_node = bench_storage.create_node(&right_path, "raisin:Folder");
            let right_id = right_node.id.clone();

            nodes_repo
                .put(
                    constants::TENANT,
                    constants::REPO,
                    constants::BRANCH,
                    constants::WORKSPACE,
                    right_node,
                )
                .await
                .expect("Failed to create right child");

            nodes_info.push((right_id.clone(), right_path.clone(), depth + 1));
            queue.push((right_path, right_id, depth + 1));
            created += 1;
        }
    }

    nodes_info
}

// ============================================================================
// Flat Structure Benchmarks
// ============================================================================

fn bench_flat_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("flat_create");
    group.sample_size(10); // Reduce sample size for faster benchmarking
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                rt.block_on(async {
                    let bench = BenchStorage::new().await;
                    let storage = bench.storage();
                    black_box(create_flat_structure(storage, count).await);
                })
            });
        });
    }
    group.finish();
}

fn bench_flat_reorder(c: &mut Criterion) {
    let mut group = c.benchmark_group("flat_reorder");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create flat structure for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        create_flat_structure(storage, count).await;
                        bench
                    })
                },
                |bench| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let nodes = storage.nodes();
                        // List root to get actual child names
                        let children = nodes
                            .list_root(
                                constants::TENANT,
                                constants::REPO,
                                constants::BRANCH,
                                constants::WORKSPACE,
                                None,
                            )
                            .await
                            .expect("Failed to list root");

                        if children.len() >= 2 {
                            // Reorder the middle child to position 0
                            let middle_idx = children.len() / 2;
                            let child_name = &children[middle_idx].name;
                            black_box(
                                nodes
                                    .reorder_child(
                                        constants::TENANT,
                                        constants::REPO,
                                        constants::BRANCH,
                                        constants::WORKSPACE,
                                        "/",
                                        child_name,
                                        0,
                                    )
                                    .await
                                    .ok(),
                            );
                        }
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_flat_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("flat_delete");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create flat structure for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        let node_ids = create_flat_structure(storage, count).await;
                        (bench, node_ids)
                    })
                },
                |(bench, node_ids)| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let nodes = storage.nodes();
                        // Delete a node in the middle
                        let middle_idx = count / 2;
                        black_box(
                            nodes
                                .delete(
                                    constants::TENANT,
                                    constants::REPO,
                                    constants::BRANCH,
                                    constants::WORKSPACE,
                                    &node_ids[middle_idx],
                                )
                                .await
                                .expect("Failed to delete"),
                        );
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_flat_branch_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("flat_branch_create");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create flat structure for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        create_flat_structure(storage, count).await;
                        bench
                    })
                },
                |bench| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let branches = storage.branches();
                        let branch_name = format!("branch-{}", uuid::Uuid::new_v4());
                        black_box(
                            branches
                                .create_branch(
                                    constants::TENANT,
                                    constants::REPO,
                                    &branch_name,
                                    "bench-user",
                                    None,
                                    false,
                                )
                                .await
                                .expect("Failed to create branch"),
                        );
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_flat_list_root(c: &mut Criterion) {
    let mut group = c.benchmark_group("flat_list_root");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create flat structure for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        create_flat_structure(storage, count).await;
                        bench
                    })
                },
                |bench| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let nodes = storage.nodes();
                        black_box(
                            nodes
                                .list_root(
                                    constants::TENANT,
                                    constants::REPO,
                                    constants::BRANCH,
                                    constants::WORKSPACE,
                                    None,
                                )
                                .await
                                .expect("Failed to list root"),
                        );
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

// ============================================================================
// Binary Tree Benchmarks
// ============================================================================

fn bench_tree_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_create");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                rt.block_on(async {
                    let bench = BenchStorage::new().await;
                    let storage = bench.storage();
                    black_box(create_binary_tree(storage, count).await);
                })
            });
        });
    }
    group.finish();
}

fn bench_tree_reorder(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_reorder");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create binary tree for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        create_binary_tree(storage, count).await;
                        bench
                    })
                },
                |bench| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let nodes = storage.nodes();
                        // Reorder left child to position 1 (swap with right)
                        black_box(
                            nodes
                                .reorder_child(
                                    constants::TENANT,
                                    constants::REPO,
                                    constants::BRANCH,
                                    constants::WORKSPACE,
                                    "/tree-root",
                                    "l000000",
                                    1,
                                )
                                .await
                                .ok(), // Ignore errors if child doesn't exist
                        );
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_tree_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_delete");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create binary tree for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        let nodes_info = create_binary_tree(storage, count).await;
                        (bench, nodes_info)
                    })
                },
                |(bench, nodes_info)| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let nodes = storage.nodes();
                        // Delete a leaf node (last node in the tree)
                        if let Some((node_id, _, _)) = nodes_info.last() {
                            black_box(
                                nodes
                                    .delete(
                                        constants::TENANT,
                                        constants::REPO,
                                        constants::BRANCH,
                                        constants::WORKSPACE,
                                        node_id,
                                    )
                                    .await
                                    .expect("Failed to delete"),
                            );
                        }
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_tree_branch_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_branch_create");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create binary tree for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        create_binary_tree(storage, count).await;
                        bench
                    })
                },
                |bench| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let branches = storage.branches();
                        let branch_name = format!("tree-branch-{}", uuid::Uuid::new_v4());
                        black_box(
                            branches
                                .create_branch(
                                    constants::TENANT,
                                    constants::REPO,
                                    &branch_name,
                                    "bench-user",
                                    None,
                                    false,
                                )
                                .await
                                .expect("Failed to create branch"),
                        );
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_tree_list_children(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_list_children");
    group.sample_size(10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &count in BENCH_SIZES {
        group.throughput(Throughput::Elements(2)); // Root has 2 children
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    // Setup: create binary tree for each iteration
                    rt.block_on(async {
                        let bench = BenchStorage::new().await;
                        let storage = bench.storage();
                        create_binary_tree(storage, count).await;
                        bench
                    })
                },
                |bench| {
                    rt.block_on(async {
                        let storage = bench.storage();
                        let nodes = storage.nodes();
                        black_box(
                            nodes
                                .list_children(
                                    constants::TENANT,
                                    constants::REPO,
                                    constants::BRANCH,
                                    constants::WORKSPACE,
                                    "/tree-root",
                                    None,
                                )
                                .await
                                .expect("Failed to list children"),
                        );
                    })
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    flat_benches,
    bench_flat_create,
    bench_flat_reorder,
    bench_flat_delete,
    bench_flat_branch_create,
    bench_flat_list_root,
);

criterion_group!(
    tree_benches,
    bench_tree_create,
    bench_tree_reorder,
    bench_tree_delete,
    bench_tree_branch_create,
    bench_tree_list_children,
);

criterion_main!(flat_benches, tree_benches);
