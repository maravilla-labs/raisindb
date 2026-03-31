//! Performance benchmarks for VectorClock operations
//!
//! This benchmark suite measures:
//! - Basic operations (increment, get, merge) performance
//! - Comparison operations at different cluster sizes
//! - Serialization overhead (JSON and MessagePack)
//! - Memory overhead per operation
//! - Realistic replication workloads
//!
//! Target cluster sizes: 3, 10, 20, 50, 100 nodes

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use raisin_replication::vector_clock::VectorClock;
use std::collections::HashMap;

/// Helper to create a vector clock with N nodes
fn create_vector_clock_with_nodes(num_nodes: usize, counter_value: u64) -> VectorClock {
    let mut vc = VectorClock::new();
    for i in 0..num_nodes {
        vc.set(&format!("node{}", i), counter_value);
    }
    vc
}

/// Benchmark increment operations at different cluster sizes
fn bench_increment(c: &mut Criterion) {
    let mut group = c.benchmark_group("increment");

    for num_nodes in [3, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                let mut vc = create_vector_clock_with_nodes(num_nodes, 0);
                let mut counter = 0;
                b.iter(|| {
                    let node_id = format!("node{}", counter % num_nodes);
                    black_box(vc.increment(&node_id));
                    counter += 1;
                });
            },
        );
    }

    group.finish();
}

/// Benchmark get operations at different cluster sizes
fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("get");

    for num_nodes in [3, 10, 20, 50, 100] {
        let vc = create_vector_clock_with_nodes(num_nodes, 42);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                let mut counter = 0;
                b.iter(|| {
                    let node_id = format!("node{}", counter % num_nodes);
                    black_box(vc.get(&node_id));
                    counter += 1;
                });
            },
        );
    }

    group.finish();
}

/// Benchmark comparison operations (happens_before, concurrent, etc.)
fn bench_compare(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare");

    for num_nodes in [3, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes_happens_before", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                let vc1 = create_vector_clock_with_nodes(num_nodes, 10);
                let vc2 = create_vector_clock_with_nodes(num_nodes, 20);
                b.iter(|| {
                    black_box(vc1.compare(&vc2));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes_concurrent", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                // Create concurrent clocks: vc1 has node0=10, vc2 has node1=10
                let mut vc1 = VectorClock::new();
                vc1.set("node0", 10);
                let mut vc2 = VectorClock::new();
                vc2.set("node1", 10);
                b.iter(|| {
                    black_box(vc1.compare(&vc2));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark merge operations at different cluster sizes
fn bench_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("merge");

    for num_nodes in [3, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                let vc1 = create_vector_clock_with_nodes(num_nodes, 10);
                let vc2 = create_vector_clock_with_nodes(num_nodes, 20);
                b.iter(|| {
                    let mut vc = vc1.clone();
                    vc.merge(&vc2);
                    black_box(vc);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark distance calculations at different cluster sizes
fn bench_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("distance");

    for num_nodes in [3, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                let vc1 = create_vector_clock_with_nodes(num_nodes, 10);
                let vc2 = create_vector_clock_with_nodes(num_nodes, 100);
                b.iter(|| {
                    black_box(vc1.distance(&vc2));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark JSON serialization at different cluster sizes
fn bench_json_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_serialization");

    for num_nodes in [3, 10, 20, 50, 100] {
        let vc = create_vector_clock_with_nodes(num_nodes, 12345);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes_serialize", num_nodes)),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    black_box(serde_json::to_string(&vc).unwrap());
                });
            },
        );

        let json = serde_json::to_string(&vc).unwrap();
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes_deserialize", num_nodes)),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    black_box(serde_json::from_str::<VectorClock>(&json).unwrap());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark MessagePack serialization at different cluster sizes
fn bench_msgpack_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("msgpack_serialization");

    for num_nodes in [3, 10, 20, 50, 100] {
        let vc = create_vector_clock_with_nodes(num_nodes, 12345);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes_serialize", num_nodes)),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    black_box(rmp_serde::to_vec(&vc).unwrap());
                });
            },
        );

        let msgpack = rmp_serde::to_vec(&vc).unwrap();
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes_deserialize", num_nodes)),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    black_box(rmp_serde::from_slice::<VectorClock>(&msgpack).unwrap());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark memory overhead by measuring serialized size
fn bench_memory_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_overhead");

    println!("\n=== Vector Clock Memory Overhead Analysis ===");
    println!(
        "{:<15} {:<20} {:<20} {:<15}",
        "Cluster Size", "JSON Size (bytes)", "MsgPack Size (bytes)", "Bytes/Node"
    );
    println!("{:-<70}", "");

    for num_nodes in [3, 10, 20, 50, 100] {
        let vc = create_vector_clock_with_nodes(num_nodes, 12345);

        // Measure JSON size
        let json = serde_json::to_string(&vc).unwrap();
        let json_size = json.len();

        // Measure MessagePack size
        let msgpack = rmp_serde::to_vec(&vc).unwrap();
        let msgpack_size = msgpack.len();

        let bytes_per_node = msgpack_size / num_nodes;

        println!(
            "{:<15} {:<20} {:<20} {:<15}",
            num_nodes, json_size, msgpack_size, bytes_per_node
        );

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    let vc = black_box(&vc);
                    black_box(vc.clone());
                });
            },
        );
    }

    println!("{:-<70}\n", "");

    group.finish();
}

/// Benchmark realistic replication scenario
/// Simulates: receiving operation, merging vector clock, comparing for conflicts
fn bench_realistic_replication(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_replication");
    group.throughput(Throughput::Elements(1000));

    for num_nodes in [3, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                let mut local_vc = create_vector_clock_with_nodes(num_nodes, 100);

                b.iter(|| {
                    for i in 0..1000 {
                        // Simulate receiving an operation from a peer
                        let mut remote_vc = create_vector_clock_with_nodes(num_nodes, 100);
                        remote_vc.increment(&format!("node{}", i % num_nodes));

                        // Compare to detect conflicts
                        let ordering = local_vc.compare(&remote_vc);
                        black_box(ordering);

                        // Merge remote clock
                        local_vc.merge(&remote_vc);

                        // Increment local counter
                        local_vc.increment(&format!("node{}", i % num_nodes));
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark catch-up scenario: calculating distance and determining lag
fn bench_catch_up_scenario(c: &mut Criterion) {
    let mut group = c.benchmark_group("catch_up_scenario");

    for num_nodes in [3, 10, 20, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &num_nodes| {
                // Local node is behind
                let local_vc = create_vector_clock_with_nodes(num_nodes, 100);
                // Remote node is ahead
                let remote_vc = create_vector_clock_with_nodes(num_nodes, 200);

                b.iter(|| {
                    // Calculate how far behind we are
                    let distance = local_vc.distance(&remote_vc);
                    black_box(distance);

                    // Check if we're behind
                    let is_behind = local_vc.happens_before(&remote_vc);
                    black_box(is_behind);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark clone operation at different cluster sizes
fn bench_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone");

    for num_nodes in [3, 10, 20, 50, 100] {
        let vc = create_vector_clock_with_nodes(num_nodes, 12345);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    black_box(vc.clone());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark sparse vector clocks (only a few active nodes out of many possible)
fn bench_sparse_clocks(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparse_clocks");

    // Simulate a 100-node cluster where only 5 nodes have been active
    let mut vc = VectorClock::new();
    for i in [0, 10, 25, 50, 99] {
        vc.set(&format!("node{}", i), 100);
    }

    group.bench_function("sparse_100_nodes_5_active", |b| {
        b.iter(|| {
            black_box(vc.clone());
        });
    });

    group.bench_function("sparse_compare", |b| {
        let mut vc2 = vc.clone();
        vc2.increment("node0");
        b.iter(|| {
            black_box(vc.compare(&vc2));
        });
    });

    group.bench_function("sparse_serialize_json", |b| {
        b.iter(|| {
            black_box(serde_json::to_string(&vc).unwrap());
        });
    });

    group.bench_function("sparse_serialize_msgpack", |b| {
        b.iter(|| {
            black_box(rmp_serde::to_vec(&vc).unwrap());
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_increment,
    bench_get,
    bench_compare,
    bench_merge,
    bench_distance,
    bench_json_serialization,
    bench_msgpack_serialization,
    bench_memory_overhead,
    bench_realistic_replication,
    bench_catch_up_scenario,
    bench_clone,
    bench_sparse_clocks,
);

criterion_main!(benches);
