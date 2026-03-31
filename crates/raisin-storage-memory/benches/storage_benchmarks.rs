//! Benchmarks for storage backend performance.
//!
//! Run with: `cargo bench --package raisin-storage-memory`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use raisin_models as models;
use raisin_storage::{CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeRepository, Storage};
use raisin_storage_memory::InMemoryStorage;
use std::sync::Arc;

const TENANT: &str = "bench-tenant";
const REPO: &str = "bench-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "bench";

fn create_test_node(id: &str, name: &str, parent: Option<String>) -> models::nodes::Node {
    models::nodes::Node {
        id: id.to_string(),
        name: name.to_string(),
        path: if let Some(ref p) = parent {
            format!("{}/{}", p, name)
        } else {
            format!("/{}", name)
        },
        node_type: "page".to_string(),
        parent,
        workspace: Some(WORKSPACE.to_string()),
        ..Default::default()
    }
}

fn setup_storage_with_nodes(node_count: usize) -> Arc<InMemoryStorage> {
    let storage = Arc::new(InMemoryStorage::default());
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        // Create nodes
        for i in 0..node_count {
            let node = create_test_node(&format!("node{}", i), &format!("Node {}", i), None);
            storage
                .nodes()
                .create(
                    TENANT,
                    REPO,
                    BRANCH,
                    WORKSPACE,
                    node,
                    CreateNodeOptions::default(),
                )
                .await
                .unwrap();
        }
    });

    storage
}

fn bench_node_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_get");

    for node_count in [10, 100, 1000].iter() {
        let storage = setup_storage_with_nodes(*node_count);
        let rt = tokio::runtime::Runtime::new().unwrap();

        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        black_box(
                            storage
                                .nodes()
                                .get(TENANT, REPO, BRANCH, WORKSPACE, "node50", None)
                                .await
                                .unwrap(),
                        )
                    })
                });
            },
        );
    }
    group.finish();
}

fn bench_node_put(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_put");
    let rt = tokio::runtime::Runtime::new().unwrap();

    for node_count in [10, 100, 1000].iter() {
        let storage = setup_storage_with_nodes(*node_count);

        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, _| {
                let mut counter = 0;
                b.iter(|| {
                    let node = create_test_node(
                        &format!("bench_node{}", counter),
                        &format!("Bench Node {}", counter),
                        None,
                    );
                    counter += 1;
                    rt.block_on(async {
                        black_box(
                            storage
                                .nodes()
                                .create(
                                    TENANT,
                                    REPO,
                                    BRANCH,
                                    WORKSPACE,
                                    node,
                                    CreateNodeOptions::default(),
                                )
                                .await
                                .unwrap(),
                        )
                    })
                });
            },
        );
    }
    group.finish();
}

fn bench_node_list_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_list_all");

    for node_count in [10, 100, 1000].iter() {
        let storage = setup_storage_with_nodes(*node_count);
        let rt = tokio::runtime::Runtime::new().unwrap();

        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        black_box(
                            storage
                                .nodes()
                                .list_all(TENANT, REPO, BRANCH, WORKSPACE, ListOptions::default())
                                .await
                                .unwrap(),
                        )
                    })
                });
            },
        );
    }
    group.finish();
}

fn bench_node_get_by_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_get_by_path");

    for node_count in [10, 100, 1000].iter() {
        let storage = setup_storage_with_nodes(*node_count);
        let rt = tokio::runtime::Runtime::new().unwrap();

        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        black_box(
                            storage
                                .nodes()
                                .get_by_path(TENANT, REPO, BRANCH, WORKSPACE, "/Node 50", None)
                                .await
                                .unwrap(),
                        )
                    })
                });
            },
        );
    }
    group.finish();
}

fn bench_node_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_delete");
    let rt = tokio::runtime::Runtime::new().unwrap();

    for node_count in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, _| {
                b.iter_batched(
                    || setup_storage_with_nodes(*node_count),
                    |storage| {
                        rt.block_on(async {
                            black_box(
                                storage
                                    .nodes()
                                    .delete(
                                        TENANT,
                                        REPO,
                                        BRANCH,
                                        WORKSPACE,
                                        "node50",
                                        DeleteNodeOptions::default(),
                                    )
                                    .await
                                    .unwrap(),
                            )
                        })
                    },
                    criterion::BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_node_tree_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_tree_ops");
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Setup a tree structure
    let storage = Arc::new(InMemoryStorage::default());
    rt.block_on(async {
        // Create parent
        let parent = create_test_node("parent", "Parent", None);
        storage
            .nodes()
            .create(
                TENANT,
                REPO,
                BRANCH,
                WORKSPACE,
                parent,
                CreateNodeOptions::default(),
            )
            .await
            .unwrap();

        // Create children
        for i in 0..10 {
            let child = create_test_node(
                &format!("child{}", i),
                &format!("Child {}", i),
                Some("/Parent".to_string()),
            );
            storage
                .nodes()
                .create(
                    TENANT,
                    REPO,
                    BRANCH,
                    WORKSPACE,
                    child,
                    CreateNodeOptions::default(),
                )
                .await
                .unwrap();
        }
    });

    group.bench_function("list_children", |b| {
        b.iter(|| {
            rt.block_on(async {
                black_box(
                    storage
                        .nodes()
                        .list_children(
                            TENANT,
                            REPO,
                            BRANCH,
                            WORKSPACE,
                            "/Parent",
                            ListOptions::default(),
                        )
                        .await
                        .unwrap(),
                )
            })
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_node_get,
    bench_node_put,
    bench_node_list_all,
    bench_node_get_by_path,
    bench_node_delete,
    bench_node_tree_operations,
);
criterion_main!(benches);
