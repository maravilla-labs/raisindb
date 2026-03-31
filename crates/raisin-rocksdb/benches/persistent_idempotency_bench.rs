//! Performance Benchmarks for Persistent Idempotency Tracker
//!
//! This benchmark suite compares the performance of in-memory vs persistent
//! idempotency tracking, which is critical for CRDT replication correctness.
//!
//! Run with: cargo bench --bench persistent_idempotency_bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use raisin_replication::{IdempotencyTracker, InMemoryIdempotencyTracker};
use raisin_rocksdb::replication::PersistentIdempotencyTracker;
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_db() -> (TempDir, Arc<DB>) {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let cf_descriptor = ColumnFamilyDescriptor::new("applied_ops", Options::default());

    let db = DB::open_cf_descriptors(&opts, path, vec![cf_descriptor]).unwrap();

    (temp_dir, Arc::new(db))
}

// ============================================================================
// Comparison Benchmarks: In-Memory vs Persistent
// ============================================================================

fn bench_is_applied_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("is_applied_comparison");

    for tracked_count in [1_000, 10_000, 100_000] {
        // Pre-populate both trackers
        let op_ids: Vec<Uuid> = (0..tracked_count).map(|_| Uuid::new_v4()).collect();

        // In-memory tracker
        let mut in_memory = InMemoryIdempotencyTracker::new();
        for op_id in &op_ids {
            in_memory.mark_applied(op_id, 1000).unwrap();
        }

        // Persistent tracker
        let (_temp_dir, db) = create_test_db();
        let mut persistent = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());
        for op_id in &op_ids {
            persistent.mark_applied(op_id, 1000).unwrap();
        }

        group.throughput(Throughput::Elements(1));

        // Benchmark in-memory lookup
        group.bench_with_input(
            BenchmarkId::new("in_memory", tracked_count),
            &tracked_count,
            |b, _| {
                let mut counter = 0;
                b.iter(|| {
                    let op_id = &op_ids[counter % op_ids.len()];
                    let result = in_memory.is_applied(op_id);
                    black_box(result);
                    counter += 1;
                });
            },
        );

        // Benchmark persistent lookup
        group.bench_with_input(
            BenchmarkId::new("persistent", tracked_count),
            &tracked_count,
            |b, _| {
                let mut counter = 0;
                b.iter(|| {
                    let op_id = &op_ids[counter % op_ids.len()];
                    let result = persistent.is_applied(op_id);
                    black_box(result);
                    counter += 1;
                });
            },
        );
    }

    group.finish();
}

fn bench_mark_applied_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("mark_applied_comparison");

    group.throughput(Throughput::Elements(1));

    // In-memory single mark
    group.bench_function("in_memory_single", |b| {
        let mut tracker = InMemoryIdempotencyTracker::new();
        b.iter(|| {
            let op_id = Uuid::new_v4();
            tracker.mark_applied(&op_id, 1000).unwrap();
            black_box(&tracker);
        });
    });

    // Persistent single mark
    group.bench_function("persistent_single", |b| {
        let (_temp_dir, db) = create_test_db();
        let mut tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

        b.iter(|| {
            let op_id = Uuid::new_v4();
            tracker.mark_applied(&op_id, 1000).unwrap();
            black_box(&tracker);
        });
    });

    group.finish();
}

fn bench_batch_mark_applied_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_mark_applied_comparison");

    for batch_size in [10, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));

        // In-memory batch
        group.bench_with_input(
            BenchmarkId::new("in_memory", batch_size),
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

        // Persistent batch
        group.bench_with_input(
            BenchmarkId::new("persistent", batch_size),
            &batch_size,
            |b, &size| {
                let (_temp_dir, db) = create_test_db();

                b.iter(|| {
                    let tracker =
                        PersistentIdempotencyTracker::new(db.clone(), "applied_ops".to_string());
                    let ops: Vec<(Uuid, u64)> =
                        (0..size).map(|i| (Uuid::new_v4(), 1000 + i)).collect();

                    tracker.mark_applied_batch(ops.into_iter()).unwrap();
                    black_box(tracker);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Persistent-Only Benchmarks
// ============================================================================

fn bench_persistent_load_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("persistent_load_all");

    for count in [1_000, 10_000, 100_000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            // Pre-populate
            let (_temp_dir, db) = create_test_db();
            let mut tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

            for i in 0..count {
                tracker.mark_applied(&Uuid::new_v4(), 1000 + i).unwrap();
            }

            b.iter(|| {
                let loaded = tracker.load_all_applied();
                let _ = black_box(loaded);
            });
        });
    }

    group.finish();
}

fn bench_persistent_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("persistent_count");

    for count in [1_000, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            // Pre-populate
            let (_temp_dir, db) = create_test_db();
            let mut tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

            for i in 0..count {
                tracker.mark_applied(&Uuid::new_v4(), 1000 + i).unwrap();
            }

            b.iter(|| {
                let count = tracker.count_applied();
                let _ = black_box(count);
            });
        });
    }

    group.finish();
}

fn bench_persistent_gc(c: &mut Criterion) {
    let mut group = c.benchmark_group("persistent_gc");

    for count in [1_000, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter(|| {
                // Pre-populate with old and new operations
                let (_temp_dir, db) = create_test_db();
                let mut tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

                // Half old, half new
                for i in 0..count / 2 {
                    tracker.mark_applied(&Uuid::new_v4(), 1000 + i).unwrap();
                }
                for i in count / 2..count {
                    tracker.mark_applied(&Uuid::new_v4(), 100_000 + i).unwrap();
                }

                // GC operations older than 50,000
                let current_time = 110_000;
                let ttl = 60_000;

                let removed = tracker.gc_old_operations(current_time, ttl);
                let _ = black_box(removed);
            });
        });
    }

    group.finish();
}

// ============================================================================
// Realistic Workload Benchmarks
// ============================================================================

fn bench_realistic_catch_up_scenario(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_catch_up");

    for op_count in [100, 1000, 10000] {
        group.throughput(Throughput::Elements(op_count as u64));

        // In-memory scenario
        group.bench_with_input(
            BenchmarkId::new("in_memory", op_count),
            &op_count,
            |b, &count| {
                b.iter(|| {
                    let mut tracker = InMemoryIdempotencyTracker::new();

                    // Simulate catch-up: check and mark operations
                    for i in 0..count {
                        let op_id = Uuid::new_v4();

                        // Check if applied (should be false)
                        let is_applied = tracker.is_applied(&op_id).unwrap();
                        black_box(is_applied);

                        // Mark as applied
                        tracker.mark_applied(&op_id, 1000 + i).unwrap();
                    }

                    black_box(tracker);
                });
            },
        );

        // Persistent scenario
        group.bench_with_input(
            BenchmarkId::new("persistent", op_count),
            &op_count,
            |b, &count| {
                let (_temp_dir, db) = create_test_db();

                b.iter(|| {
                    let tracker =
                        PersistentIdempotencyTracker::new(db.clone(), "applied_ops".to_string());

                    // Simulate catch-up: check and mark operations
                    for i in 0..count {
                        let op_id = Uuid::new_v4();

                        // Check if applied (should be false)
                        let is_applied = tracker.is_applied(&op_id).unwrap();
                        black_box(is_applied);

                        // Mark as applied
                        tracker.mark_applied(&op_id, 1000 + i).unwrap();
                    }

                    black_box(tracker);
                });
            },
        );
    }

    group.finish();
}

fn bench_realistic_normal_operation(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_normal_operation");

    // Simulate: 90% check (hit), 10% mark (miss)
    for total_ops in [1000, 10000] {
        let tracked_ops = total_ops * 9 / 10; // 90% already tracked

        group.throughput(Throughput::Elements(total_ops as u64));

        // In-memory scenario
        group.bench_with_input(
            BenchmarkId::new("in_memory", total_ops),
            &total_ops,
            |b, &total_ops| {
                // Pre-populate
                let mut tracker = InMemoryIdempotencyTracker::new();
                let existing_ops: Vec<Uuid> = (0..tracked_ops).map(|_| Uuid::new_v4()).collect();

                for op_id in &existing_ops {
                    tracker.mark_applied(op_id, 1000).unwrap();
                }

                b.iter(|| {
                    let mut counter = 0;

                    for i in 0..total_ops {
                        if i < tracked_ops {
                            // Check existing (hit)
                            let op_id = &existing_ops[counter % existing_ops.len()];
                            let _ = tracker.is_applied(op_id).unwrap();
                            counter += 1;
                        } else {
                            // Mark new (miss)
                            tracker.mark_applied(&Uuid::new_v4(), 1000 + i).unwrap();
                        }
                    }

                    black_box(&tracker);
                });
            },
        );

        // Persistent scenario
        group.bench_with_input(
            BenchmarkId::new("persistent", total_ops),
            &total_ops,
            |b, &total_ops| {
                // Pre-populate
                let (_temp_dir, db) = create_test_db();
                let tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());
                let existing_ops: Vec<Uuid> = (0..tracked_ops).map(|_| Uuid::new_v4()).collect();

                for op_id in &existing_ops {
                    tracker.mark_applied(op_id, 1000).unwrap();
                }

                b.iter(|| {
                    let mut counter = 0;

                    for i in 0..total_ops {
                        if i < tracked_ops {
                            // Check existing (hit)
                            let op_id = &existing_ops[counter % existing_ops.len()];
                            let _ = tracker.is_applied(op_id).unwrap();
                            counter += 1;
                        } else {
                            // Mark new (miss)
                            tracker.mark_applied(&Uuid::new_v4(), 1000 + i).unwrap();
                        }
                    }

                    black_box(&tracker);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Performance Summary
// ============================================================================

fn bench_performance_summary(c: &mut Criterion) {
    let group = c.benchmark_group("performance_summary");

    println!("\n=== Idempotency Tracker Performance Summary ===");
    println!(
        "{:<30} {:<20} {:<20}",
        "Operation", "In-Memory (µs)", "Persistent (µs)"
    );
    println!("{:-<70}", "");

    // Note: Actual timing is done by criterion, this is just for display
    // The real numbers will be in the benchmark output

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    comparison_benches,
    bench_is_applied_comparison,
    bench_mark_applied_comparison,
    bench_batch_mark_applied_comparison,
);

criterion_group!(
    persistent_benches,
    bench_persistent_load_all,
    bench_persistent_count,
    bench_persistent_gc,
);

criterion_group!(
    realistic_benches,
    bench_realistic_catch_up_scenario,
    bench_realistic_normal_operation,
);

criterion_group!(summary_benches, bench_performance_summary,);

criterion_main!(
    comparison_benches,
    persistent_benches,
    realistic_benches,
    summary_benches,
);
