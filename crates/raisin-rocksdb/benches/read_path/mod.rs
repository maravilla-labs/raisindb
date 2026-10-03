//! Read-path benchmarks: point reads as a node's history and a parent's child
//! count grow.
//!
//! - `get_by_id/{revs}` — HEAD read of a node edited `revs` times.
//! - `get_at_old_revision/{revs}` — read of the same node as of its FIRST
//!   version. A walking time-travel read grows with `revs`; a seek does not.
//! - `get_by_path/{children}` — HEAD read by path of a parent with `children`
//!   children, each edited a few times. Pays the `has_children` probe, which
//!   must not grow with the child count.
//!
//! Lives under `benches/read_path/mod.rs` (not `benches/read_path.rs`) so Cargo
//! does not auto-discover it as a bench target of its own.

use super::{constants, BenchStorage};
use criterion::{black_box, BenchmarkId, Criterion};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, Storage, UpdateNodeOptions,
};

const REVISION_COUNTS: &[usize] = &[1, 10, 100];
const CHILD_COUNTS: &[usize] = &[10, 100, 1000];
const EDITS_PER_CHILD: usize = 3;

async fn head(storage: &RocksDBStorage) -> HLC {
    storage
        .branches()
        .get_head(constants::TENANT, constants::REPO, constants::BRANCH)
        .await
        .expect("Failed to read HEAD")
}

/// Create `path` and update it until it has `revs` versions. Returns the id
/// and the revision of its first version.
async fn create_edited(bench: &BenchStorage, path: &str, revs: usize) -> (String, HLC) {
    let storage = bench.storage();
    let nodes = storage.nodes();
    let mut node = bench.create_node(path, "raisin:Page");
    let id = node.id.clone();
    nodes
        .create(constants::SCOPE, node.clone(), CreateNodeOptions::default())
        .await
        .expect("Failed to create node");
    let first = head(storage).await;

    for i in 1..revs {
        node.properties
            .insert("title".to_string(), PropertyValue::String(format!("v{i}")));
        nodes
            .update(constants::SCOPE, node.clone(), UpdateNodeOptions::default())
            .await
            .expect("Failed to update node");
    }
    (id, first)
}

fn bench_get_by_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_by_id");
    group.sample_size(20);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &revs in REVISION_COUNTS {
        let (bench, id) = rt.block_on(async {
            let bench = BenchStorage::new().await;
            let (id, _) = create_edited(&bench, "/edited", revs).await;
            (bench, id)
        });
        group.bench_with_input(BenchmarkId::from_parameter(revs), &revs, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    black_box(
                        bench
                            .storage()
                            .nodes()
                            .get(constants::SCOPE, &id, None)
                            .await
                            .expect("Failed to get node"),
                    );
                })
            });
        });
    }
    group.finish();
}

fn bench_get_at_old_revision(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_at_old_revision");
    group.sample_size(20);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &revs in REVISION_COUNTS {
        let (bench, id, first) = rt.block_on(async {
            let bench = BenchStorage::new().await;
            let (id, first) = create_edited(&bench, "/edited", revs).await;
            (bench, id, first)
        });
        group.bench_with_input(BenchmarkId::from_parameter(revs), &revs, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    black_box(
                        bench
                            .storage()
                            .nodes()
                            .get(constants::SCOPE, &id, Some(&first))
                            .await
                            .expect("Failed to get node"),
                    );
                })
            });
        });
    }
    group.finish();
}

fn bench_get_by_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_by_path");
    group.sample_size(20);
    let rt = tokio::runtime::Runtime::new().unwrap();

    for &children in CHILD_COUNTS {
        let bench = rt.block_on(async {
            let bench = BenchStorage::new().await;
            bench
                .storage()
                .nodes()
                .create(
                    constants::SCOPE,
                    bench.create_node("/parent", "raisin:Page"),
                    CreateNodeOptions::default(),
                )
                .await
                .expect("Failed to create parent");
            for i in 0..children {
                create_edited(&bench, &format!("/parent/child{i:05}"), EDITS_PER_CHILD).await;
            }
            bench
        });
        group.bench_with_input(BenchmarkId::from_parameter(children), &children, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    black_box(
                        bench
                            .storage()
                            .nodes()
                            .get_by_path(constants::SCOPE, "/parent", None)
                            .await
                            .expect("Failed to get parent"),
                    );
                })
            });
        });
    }
    group.finish();
}

/// Every read-path benchmark, for the `criterion_group!` in the bench root.
pub fn read_path_benches(c: &mut Criterion) {
    bench_get_by_id(c);
    bench_get_at_old_revision(c);
    bench_get_by_path(c);
}
