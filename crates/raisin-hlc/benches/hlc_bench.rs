// SPDX-License-Identifier: BSL-1.1

//! Performance benchmarks for HLC operations
//!
//! Target performance metrics:
//! - tick(): <100ns per operation
//! - update(): <200ns per operation
//! - encode_descending(): <50ns per operation
//! - decode_descending(): <50ns per operation

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use raisin_hlc::{NodeHLCState, HLC};
use std::sync::Arc;
use std::thread;

/// Benchmark single-threaded tick() performance
fn bench_tick_single_thread(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_single_thread");

    let state = NodeHLCState::new("bench-node".to_string());

    group.bench_function("tick", |b| {
        b.iter(|| {
            black_box(state.tick());
        });
    });

    group.finish();
}

/// Benchmark concurrent tick() performance
fn bench_tick_concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_concurrent");

    for num_threads in [2, 4, 8, 16] {
        group.throughput(Throughput::Elements(10_000 * num_threads));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_threads", num_threads)),
            &num_threads,
            |b, &num_threads| {
                let state = Arc::new(NodeHLCState::new("bench-node".to_string()));

                b.iter(|| {
                    let mut handles = vec![];

                    for _ in 0..num_threads {
                        let state = Arc::clone(&state);
                        let handle = thread::spawn(move || {
                            for _ in 0..10_000 {
                                black_box(state.tick());
                            }
                        });
                        handles.push(handle);
                    }

                    for handle in handles {
                        handle.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark update() from remote timestamps
fn bench_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("update");

    let state = NodeHLCState::new("bench-node".to_string());
    let remote = HLC::new(1000, 42);

    group.bench_function("update", |b| {
        b.iter(|| {
            black_box(state.update(black_box(&remote)));
        });
    });

    group.finish();
}

/// Benchmark mixed tick() and update() operations
fn bench_mixed_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_operations");

    let state = NodeHLCState::new("bench-node".to_string());
    let remote = HLC::new(1000, 42);

    group.bench_function("50_tick_50_update", |b| {
        b.iter(|| {
            for i in 0..100 {
                if i % 2 == 0 {
                    black_box(state.tick());
                } else {
                    black_box(state.update(black_box(&remote)));
                }
            }
        });
    });

    group.finish();
}

/// Benchmark HLC encoding performance
fn bench_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("encoding");

    let hlc = HLC::new(1705843009213693952, 42);

    group.bench_function("encode_descending", |b| {
        b.iter(|| {
            black_box(black_box(&hlc).encode_descending());
        });
    });

    let encoded = hlc.encode_descending();

    group.bench_function("decode_descending", |b| {
        b.iter(|| {
            black_box(HLC::decode_descending(black_box(&encoded)).unwrap());
        });
    });

    group.bench_function("encode_decode_roundtrip", |b| {
        b.iter(|| {
            let encoded = black_box(&hlc).encode_descending();
            black_box(HLC::decode_descending(&encoded).unwrap());
        });
    });

    group.finish();
}

/// Benchmark string serialization performance
fn bench_string_format(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_format");

    let hlc = HLC::new(1705843009213693952, 42);

    group.bench_function("to_string", |b| {
        b.iter(|| {
            black_box(black_box(&hlc).to_string());
        });
    });

    let string = hlc.to_string();

    group.bench_function("from_str", |b| {
        b.iter(|| {
            black_box(black_box(&string).parse::<HLC>().unwrap());
        });
    });

    group.bench_function("string_roundtrip", |b| {
        b.iter(|| {
            let s = black_box(&hlc).to_string();
            black_box(s.parse::<HLC>().unwrap());
        });
    });

    group.finish();
}

/// Benchmark comparison operations
fn bench_comparisons(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparisons");

    let hlc1 = HLC::new(1000, 0);
    let hlc2 = HLC::new(1000, 1);

    group.bench_function("equality", |b| {
        b.iter(|| {
            black_box(black_box(&hlc1) == black_box(&hlc2));
        });
    });

    group.bench_function("ordering", |b| {
        b.iter(|| {
            black_box(black_box(&hlc1).cmp(black_box(&hlc2)));
        });
    });

    group.finish();
}

/// Benchmark u128 conversion
fn bench_u128_conversion(c: &mut Criterion) {
    let mut group = c.benchmark_group("u128_conversion");

    let hlc = HLC::new(1705843009213693952, 42);

    group.bench_function("as_u128", |b| {
        b.iter(|| {
            black_box(black_box(&hlc).as_u128());
        });
    });

    let numeric = hlc.as_u128();

    group.bench_function("from_u128", |b| {
        b.iter(|| {
            black_box(HLC::from_u128(black_box(numeric)));
        });
    });

    group.finish();
}

/// Benchmark realistic workload: 90% reads (tick), 10% writes (update)
fn bench_realistic_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_workload");
    group.throughput(Throughput::Elements(1000));

    let state = Arc::new(NodeHLCState::new("bench-node".to_string()));

    group.bench_function("90_tick_10_update", |b| {
        let mut counter = 0u64;
        b.iter(|| {
            for _ in 0..1000 {
                if counter % 10 == 0 {
                    // 10% updates
                    let remote = HLC::new(1000 + counter, 0);
                    black_box(state.update(&remote));
                } else {
                    // 90% ticks
                    black_box(state.tick());
                }
                counter += 1;
            }
        });
    });

    group.finish();
}

/// Benchmark comparison with u64 counter (baseline)
fn bench_u64_comparison(c: &mut Criterion) {
    use std::sync::atomic::{AtomicU64, Ordering};

    let mut group = c.benchmark_group("u64_baseline");

    // Baseline: atomic u64 increment
    let counter = AtomicU64::new(0);
    group.bench_function("atomic_u64_increment", |b| {
        b.iter(|| {
            black_box(counter.fetch_add(1, Ordering::AcqRel));
        });
    });

    // HLC tick for comparison
    let state = NodeHLCState::new("bench-node".to_string());
    group.bench_function("hlc_tick", |b| {
        b.iter(|| {
            black_box(state.tick());
        });
    });

    group.finish();
}

/// Benchmark sorting performance
fn bench_sorting(c: &mut Criterion) {
    let mut group = c.benchmark_group("sorting");

    // Generate test data
    let hlcs: Vec<HLC> = (0..1000)
        .map(|i| HLC::new(1000 + (i % 100), i % 50))
        .collect();

    group.bench_function("sort_1000_hlcs", |b| {
        b.iter(|| {
            let mut data = hlcs.clone();
            data.sort();
            black_box(data);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_tick_single_thread,
    bench_tick_concurrent,
    bench_update,
    bench_mixed_operations,
    bench_encoding,
    bench_string_format,
    bench_comparisons,
    bench_u128_conversion,
    bench_realistic_workload,
    bench_u64_comparison,
    bench_sorting,
);

criterion_main!(benches);
