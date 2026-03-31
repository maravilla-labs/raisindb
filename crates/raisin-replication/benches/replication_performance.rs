//! Comprehensive Performance Benchmarks for CRDT Replication System
//!
//! This benchmark suite measures the performance of the CRDT replication system
//! including:
//! - Causal Delivery Buffer (ensures happens-before ordering)
//! - Persistent Idempotency Tracker (prevents duplicate application)
//! - Operation Decomposition (breaks batched ops into atomic CRDT operations)
//! - End-to-end replication latency
//!
//! Run with: cargo bench --bench replication_performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{
    causal_delivery::CausalDeliveryBuffer,
    operation::{OpType, Operation, ReplicatedNodeChange, ReplicatedNodeChangeKind},
    operation_decomposer::decompose_operation,
    replay::{IdempotencyTracker, InMemoryIdempotencyTracker, ReplayEngine},
    VectorClock,
};
use std::collections::HashSet;
use uuid::Uuid;

// ============================================================================
// Helper Functions for Creating Test Data
// ============================================================================

/// Create a test operation with SetProperty
fn make_set_property_op(
    cluster_node_id: &str,
    op_seq: u64,
    vc: VectorClock,
    timestamp_ms: u64,
    node_id: &str,
) -> Operation {
    Operation {
        op_id: Uuid::new_v4(),
        op_seq,
        cluster_node_id: cluster_node_id.to_string(),
        timestamp_ms,
        vector_clock: vc,
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: node_id.to_string(),
            property_name: "value".to_string(),
            value: PropertyValue::Integer(op_seq as i64),
        },
        revision: None,
        actor: "benchmark".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    }
}

/// Create a test operation with AddChild
fn make_add_child_op(
    cluster_node_id: &str,
    op_seq: u64,
    vc: VectorClock,
    timestamp_ms: u64,
) -> Operation {
    Operation {
        op_id: Uuid::new_v4(),
        op_seq,
        cluster_node_id: cluster_node_id.to_string(),
        timestamp_ms,
        vector_clock: vc,
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::AddRelation {
            source_id: format!("parent_{}", op_seq),
            source_workspace: "ws".to_string(),
            relation_type: "children".to_string(),
            target_id: format!("child_{}", op_seq),
            target_workspace: "ws".to_string(),
            properties: Default::default(),
            relation: todo!(),
        },
        revision: None,
        actor: "benchmark".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    }
}

/// Create a test node for ApplyRevision
fn make_test_node(id: &str) -> Node {
    Node {
        id: id.to_string(),
        name: format!("node_{}", id),
        node_type: "Document".to_string(),
        ..Default::default()
    }
}

/// Create ApplyRevision operation with N node changes
fn make_apply_revision_op(
    cluster_node_id: &str,
    op_seq: u64,
    vc: VectorClock,
    timestamp_ms: u64,
    num_changes: usize,
) -> Operation {
    let branch_head = HLC::new(timestamp_ms, 0);

    let node_changes: Vec<ReplicatedNodeChange> = (0..num_changes)
        .map(|i| ReplicatedNodeChange {
            node: make_test_node(&format!("node_{}", i)),
            parent_id: if i > 0 {
                Some(format!("node_{}", i - 1))
            } else {
                None
            },
            kind: if i % 10 == 0 {
                ReplicatedNodeChangeKind::Delete
            } else {
                ReplicatedNodeChangeKind::Upsert
            },
            cf_order_key: todo!(),
        })
        .collect();

    Operation {
        op_id: Uuid::new_v4(),
        op_seq,
        cluster_node_id: cluster_node_id.to_string(),
        timestamp_ms,
        vector_clock: vc,
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::ApplyRevision {
            branch_head,
            node_changes,
        },
        revision: Some(branch_head),
        actor: "benchmark".to_string(),
        message: Some("Benchmark commit".to_string()),
        is_system: false,
        acknowledged_by: HashSet::new(),
    }
}

/// Create sequential operations from a single node
fn create_sequential_ops(node_id: &str, count: usize) -> Vec<Operation> {
    let mut ops = Vec::with_capacity(count);
    let mut vc = VectorClock::new();

    for i in 0..count {
        vc.increment(node_id);
        ops.push(make_set_property_op(
            node_id,
            i as u64 + 1,
            vc.clone(),
            1000 + i as u64 * 10,
            "target_node",
        ));
    }

    ops
}

// ============================================================================
// A. Operation Throughput Benchmarks
// ============================================================================

fn bench_operation_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("operation_throughput");

    for batch_size in [10, 100, 1000, 10000] {
        // SetProperty operations
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::new("set_property", batch_size),
            &batch_size,
            |b, &size| {
                let ops = create_sequential_ops("node1", size);
                let mut engine = ReplayEngine::new();

                b.iter(|| {
                    let result = engine.replay(ops.clone());
                    let _ = black_box(result);
                });
            },
        );

        // AddChild operations
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::new("add_child", batch_size),
            &batch_size,
            |b, &size| {
                let mut ops = Vec::with_capacity(size);
                let mut vc = VectorClock::new();

                for i in 0..size {
                    vc.increment("node1");
                    ops.push(make_add_child_op(
                        "node1",
                        i as u64 + 1,
                        vc.clone(),
                        1000 + i as u64 * 10,
                    ));
                }

                let mut engine = ReplayEngine::new();

                b.iter(|| {
                    let result = engine.replay(ops.clone());
                    let _ = black_box(result);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// B. Idempotency Tracker Performance Benchmarks
// ============================================================================

fn bench_idempotency_tracker_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("idempotency_lookup");

    for tracked_count in [1_000, 10_000, 100_000, 1_000_000] {
        // Create tracker with N operations already tracked
        let mut tracker = InMemoryIdempotencyTracker::new();
        let op_ids: Vec<Uuid> = (0..tracked_count).map(|_| Uuid::new_v4()).collect();

        for op_id in &op_ids {
            tracker.mark_applied(op_id, 1000).unwrap();
        }

        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("is_applied", tracked_count),
            &tracked_count,
            |b, _| {
                let mut counter = 0;
                b.iter(|| {
                    let op_id = &op_ids[counter % op_ids.len()];
                    let result = tracker.is_applied(op_id);
                    let _ = black_box(result);
                    counter += 1;
                });
            },
        );
    }

    group.finish();
}

fn bench_idempotency_tracker_mark_applied(c: &mut Criterion) {
    let mut group = c.benchmark_group("idempotency_mark_applied");

    // Single operation marking
    group.bench_function("single", |b| {
        let mut tracker = InMemoryIdempotencyTracker::new();

        b.iter(|| {
            let op_id = Uuid::new_v4();
            tracker.mark_applied(&op_id, 1000).unwrap();
            black_box(&tracker);
        });
    });

    // Batch operation marking
    for batch_size in [10, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::new("batch", batch_size),
            &batch_size,
            |b, &size| {
                b.iter(|| {
                    let mut tracker = InMemoryIdempotencyTracker::new();
                    let ops: Vec<(Uuid, u64)> =
                        (0..size).map(|i| (Uuid::new_v4(), 1000 + i)).collect();

                    tracker.mark_applied_batch(&ops).unwrap();
                    black_box(tracker);
                });
            },
        );
    }

    group.finish();
}

fn bench_idempotency_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("idempotency_memory");

    println!("\n=== Idempotency Tracker Memory Analysis ===");
    println!(
        "{:<20} {:<25} {:<20}",
        "Tracked Ops", "Approx Memory (bytes)", "Bytes/Op"
    );
    println!("{:-<65}", "");

    for count in [1_000, 10_000, 100_000, 1_000_000] {
        // Estimate: HashSet<Uuid> = ~16 bytes per UUID + overhead
        let approx_memory = count * 24; // UUID (16 bytes) + hash overhead (~8 bytes)
        let bytes_per_op = approx_memory / count;

        println!("{:<20} {:<25} {:<20}", count, approx_memory, bytes_per_op);

        group.bench_with_input(BenchmarkId::new("create", count), &count, |b, &count| {
            b.iter(|| {
                let mut tracker = InMemoryIdempotencyTracker::new();
                for i in 0..count {
                    tracker
                        .mark_applied(&Uuid::new_v4(), 1000 + i as u64)
                        .unwrap();
                }
                black_box(tracker);
            });
        });
    }

    println!("{:-<65}\n", "");
    group.finish();
}

// ============================================================================
// C. Causal Delivery Buffer Performance Benchmarks
// ============================================================================

fn bench_causal_delivery_in_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("causal_delivery_in_order");

    for batch_size in [10, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                let ops = create_sequential_ops("node1", size);

                b.iter(|| {
                    let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
                    let mut total_delivered = 0;

                    for op in &ops {
                        let delivered = buffer.deliver(op.clone());
                        total_delivered += delivered.len();
                    }

                    black_box(total_delivered);
                });
            },
        );
    }

    group.finish();
}

fn bench_causal_delivery_reversed(c: &mut Criterion) {
    let mut group = c.benchmark_group("causal_delivery_reversed");

    for batch_size in [10, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                let ops = create_sequential_ops("node1", size);

                b.iter(|| {
                    let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
                    let mut total_delivered = 0;

                    // Deliver in reverse order (worst case)
                    for op in ops.iter().rev() {
                        let delivered = buffer.deliver(op.clone());
                        total_delivered += delivered.len();
                    }

                    black_box(total_delivered);
                });
            },
        );
    }

    group.finish();
}

fn bench_causal_delivery_random(c: &mut Criterion) {
    use rand::prelude::*;

    let mut group = c.benchmark_group("causal_delivery_random");

    for batch_size in [10, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                let ops = create_sequential_ops("node1", size);

                // Pre-generate shuffled order
                let mut rng = StdRng::seed_from_u64(42);
                let mut indices: Vec<usize> = (0..size).collect();
                indices.shuffle(&mut rng);

                b.iter(|| {
                    let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
                    let mut total_delivered = 0;

                    for &idx in &indices {
                        let delivered = buffer.deliver(ops[idx].clone());
                        total_delivered += delivered.len();
                    }

                    black_box(total_delivered);
                });
            },
        );
    }

    group.finish();
}

fn bench_causal_delivery_buffer_size(c: &mut Criterion) {
    let group = c.benchmark_group("causal_delivery_buffer_growth");

    println!("\n=== Causal Delivery Buffer Size Analysis ===");
    println!(
        "{:<15} {:<20} {:<20}",
        "Total Ops", "Max Buffer Size", "Avg Buffer Size"
    );
    println!("{:-<55}", "");

    for batch_size in [10, 50, 100, 500, 1000] {
        let ops = create_sequential_ops("node1", batch_size);

        // Deliver in reverse to maximize buffer growth
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
        let mut max_buffer_size = 0;
        let mut total_buffer_size = 0;
        let mut measurements = 0;

        for op in ops.iter().rev() {
            buffer.deliver(op.clone());
            let current_size = buffer.buffer_size();
            max_buffer_size = max_buffer_size.max(current_size);
            total_buffer_size += current_size;
            measurements += 1;
        }

        let avg_buffer_size = if measurements > 0 {
            total_buffer_size / measurements
        } else {
            0
        };

        println!(
            "{:<15} {:<20} {:<20}",
            batch_size, max_buffer_size, avg_buffer_size
        );
    }

    println!("{:-<55}\n", "");
    group.finish();
}

fn bench_causal_delivery_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("causal_delivery_throughput");

    for batch_size in [100, 1000, 10000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                let ops = create_sequential_ops("node1", size);

                b.iter(|| {
                    let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
                    let mut total = 0;

                    for op in &ops {
                        let delivered = buffer.deliver(op.clone());
                        total += delivered.len();
                    }

                    black_box(total);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// D. Operation Decomposition Overhead Benchmarks
// ============================================================================

fn bench_operation_decomposition(c: &mut Criterion) {
    let mut group = c.benchmark_group("operation_decomposition");

    // Small revision (1-5 changes)
    group.bench_function("small_revision_5_changes", |b| {
        let mut vc = VectorClock::new();
        vc.increment("node1");
        let op = make_apply_revision_op("node1", 1, vc, 1000, 5);

        b.iter(|| {
            let decomposed = decompose_operation(op.clone());
            black_box(decomposed);
        });
    });

    // Medium revision (10-50 changes)
    group.bench_function("medium_revision_50_changes", |b| {
        let mut vc = VectorClock::new();
        vc.increment("node1");
        let op = make_apply_revision_op("node1", 1, vc, 1000, 50);

        b.iter(|| {
            let decomposed = decompose_operation(op.clone());
            black_box(decomposed);
        });
    });

    // Large revision (100+ changes)
    group.bench_function("large_revision_200_changes", |b| {
        let mut vc = VectorClock::new();
        vc.increment("node1");
        let op = make_apply_revision_op("node1", 1, vc, 1000, 200);

        b.iter(|| {
            let decomposed = decompose_operation(op.clone());
            black_box(decomposed);
        });
    });

    // Compare operation counts
    println!("\n=== Operation Decomposition Analysis ===");
    println!(
        "{:<20} {:<20} {:<20}",
        "Revision Size", "Original Ops", "Decomposed Ops"
    );
    println!("{:-<60}", "");

    for size in [5, 10, 50, 100, 200] {
        let mut vc = VectorClock::new();
        vc.increment("node1");
        let op = make_apply_revision_op("node1", 1, vc, 1000, size);
        let decomposed = decompose_operation(op);

        println!("{:<20} {:<20} {:<20}", size, 1, decomposed.len());
    }

    println!("{:-<60}\n", "");
    group.finish();
}

// ============================================================================
// E. End-to-End Replication Latency Benchmarks
// ============================================================================

fn bench_end_to_end_single_node_pair(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_single_node_pair");

    for op_count in [10, 100, 1000] {
        group.throughput(Throughput::Elements(op_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(op_count),
            &op_count,
            |b, &count| {
                b.iter(|| {
                    // Simulate: create op -> buffer -> replay
                    let ops = create_sequential_ops("node1", count);
                    let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
                    let mut engine = ReplayEngine::new();

                    let mut total_delivered = 0;

                    for op in ops {
                        // Causal delivery
                        let deliverable = buffer.deliver(op);

                        // Replay
                        if !deliverable.is_empty() {
                            let result = engine.replay(deliverable);
                            total_delivered += result.applied.len();
                        }
                    }

                    black_box(total_delivered);
                });
            },
        );
    }

    group.finish();
}

fn bench_catch_up_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("catch_up_replay");

    for op_count in [100, 1000, 10000] {
        group.throughput(Throughput::Elements(op_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(op_count),
            &op_count,
            |b, &count| {
                // Pre-create all operations
                let ops = create_sequential_ops("node1", count);

                b.iter(|| {
                    // Simulate catch-up: replay N operations at once
                    let mut engine = ReplayEngine::new();
                    let result = engine.replay(ops.clone());
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

fn bench_multi_node_concurrent_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_node_concurrent");

    for num_nodes in [3, 5, 10] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_nodes", num_nodes)),
            &num_nodes,
            |b, &node_count| {
                b.iter(|| {
                    // Create concurrent operations from multiple nodes
                    let mut all_ops = Vec::new();

                    for node_id in 0..node_count {
                        let node_name = format!("node{}", node_id);
                        let mut vc = VectorClock::new();

                        for i in 0..100 {
                            vc.increment(&node_name);
                            all_ops.push(make_set_property_op(
                                &node_name,
                                i + 1,
                                vc.clone(),
                                1000 + i * 10,
                                &format!("target_{}", i % 10),
                            ));
                        }
                    }

                    // Replay all operations
                    let mut engine = ReplayEngine::new();
                    let result = engine.replay(all_ops);
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(operation_benches, bench_operation_throughput,);

criterion_group!(
    idempotency_benches,
    bench_idempotency_tracker_lookup,
    bench_idempotency_tracker_mark_applied,
    bench_idempotency_memory_usage,
);

criterion_group!(
    causal_delivery_benches,
    bench_causal_delivery_in_order,
    bench_causal_delivery_reversed,
    bench_causal_delivery_random,
    bench_causal_delivery_buffer_size,
    bench_causal_delivery_throughput,
);

criterion_group!(decomposition_benches, bench_operation_decomposition,);

criterion_group!(
    e2e_benches,
    bench_end_to_end_single_node_pair,
    bench_catch_up_performance,
    bench_multi_node_concurrent_ops,
);

criterion_main!(
    operation_benches,
    idempotency_benches,
    causal_delivery_benches,
    decomposition_benches,
    e2e_benches,
);
