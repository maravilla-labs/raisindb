# Replication Performance Benchmarks

This directory contains comprehensive performance benchmarks for the CRDT replication system.

## Available Benchmarks

### 1. `replication_performance.rs` - Main Replication Benchmark Suite

Comprehensive benchmarks covering all major components of the CRDT replication system.

#### Benchmark Groups

##### A. Operation Throughput (`operation_throughput`)
Measures operations/second for different operation types:
- `set_property` - SetProperty operations at batch sizes: 10, 100, 1K, 10K
- `add_child` - AddChild/AddRelation operations at batch sizes: 10, 100, 1K, 10K

##### B. Idempotency Tracker Performance (`idempotency_*`)
- `idempotency_lookup` - Lookup performance with 1K, 10K, 100K, 1M tracked operations
- `idempotency_mark_applied` - Single and batch marking (10, 100, 1K ops)
- `idempotency_memory` - Memory usage analysis for in-memory tracker

##### C. Causal Delivery Buffer (`causal_delivery_*`)
- `causal_delivery_in_order` - Best case: operations arrive in order
- `causal_delivery_reversed` - Worst case: completely reversed operations
- `causal_delivery_random` - Realistic case: random order
- `causal_delivery_buffer_growth` - Analyzes buffer size growth patterns
- `causal_delivery_throughput` - Delivery throughput measurement

##### D. Operation Decomposition (`operation_decomposition`)
Measures decomposition overhead for ApplyRevision operations:
- Small revision: 5 node changes
- Medium revision: 50 node changes
- Large revision: 200 node changes

##### E. End-to-End Replication (`e2e_*`)
- `e2e_single_node_pair` - Single node pair replication latency
- `catch_up_replay` - Catch-up performance (100, 1K, 10K ops)
- `multi_node_concurrent` - Multi-node (3, 5, 10 nodes) concurrent operations

### 2. `vector_clock_bench.rs` - Vector Clock Performance

Benchmarks vector clock operations at different cluster sizes (3, 10, 20, 50, 100 nodes):
- Basic operations (increment, get, merge)
- Comparison operations (happens_before, concurrent)
- Serialization (JSON and MessagePack)
- Memory overhead analysis
- Realistic replication scenarios

## Running the Benchmarks

### Run All Replication Benchmarks
```bash
cd crates/raisin-replication
cargo bench
```

### Run Specific Benchmark Suite
```bash
# Main replication benchmark
cargo bench --bench replication_performance

# Vector clock benchmark
cargo bench --bench vector_clock_bench
```

### Run Specific Benchmark Group
```bash
# Only operation throughput
cargo bench --bench replication_performance -- operation_throughput

# Only causal delivery
cargo bench --bench replication_performance -- causal_delivery

# Only end-to-end benchmarks
cargo bench --bench replication_performance -- e2e
```

### Run Specific Benchmark
```bash
# Only in-order causal delivery
cargo bench --bench replication_performance -- causal_delivery_in_order

# Only operation decomposition
cargo bench --bench replication_performance -- operation_decomposition
```

### Generate Benchmark Report
```bash
# Run and save baseline
cargo bench --bench replication_performance -- --save-baseline main

# Compare against baseline
cargo bench --bench replication_performance -- --baseline main
```

## Understanding the Output

### Throughput Metrics
Operations per second for batch operations. Higher is better.
```
operation_throughput/set_property/1000
                        time:   [123.45 µs 125.67 µs 127.89 µs]
                        thrpt:  [7.82M elem/s 7.96M elem/s 8.10M elem/s]
```

### Latency Percentiles
Individual operation latency. Lower is better.
```
idempotency_lookup/10000
                        time:   [45.23 ns 46.78 ns 48.12 ns]
```

### Memory Analysis
Printed to console during benchmark run:
```
=== Idempotency Tracker Memory Analysis ===
Tracked Ops          Approx Memory (bytes)    Bytes/Op
--------------------------------------------------------------
1,000                24,000                   24
10,000               240,000                  24
```

### Buffer Size Analysis
Shows buffer growth patterns:
```
=== Causal Delivery Buffer Size Analysis ===
Total Ops        Max Buffer Size      Avg Buffer Size
-------------------------------------------------------
10               9                    4
100              99                   49
```

## Performance Targets

Based on the benchmarks, these are expected performance characteristics:

### Operation Throughput
- **SetProperty**: >100K ops/sec
- **AddChild**: >100K ops/sec
- **Batch operations**: Linear scaling up to 10K ops

### Idempotency Tracker
- **Lookup (in-memory)**: <50ns per operation
- **Mark applied**: <100ns per operation
- **Batch marking**: <10µs per 100 operations
- **Memory overhead**: ~24 bytes per tracked operation

### Causal Delivery Buffer
- **In-order delivery**: ~1-2µs per operation
- **Out-of-order delivery**: ~100µs per operation (includes buffering)
- **Buffer size**: Scales linearly with out-of-order depth
- **Throughput**: >50K ops/sec

### Operation Decomposition
- **Small revision (5 changes)**: <5µs
- **Medium revision (50 changes)**: <50µs
- **Large revision (200 changes)**: <200µs
- **Overhead**: ~1µs per node change

### End-to-End
- **Single pair replication**: <10µs per operation
- **Catch-up (1K ops)**: <10ms
- **Multi-node (10 nodes)**: <100µs per operation

## Interpreting Results

### What to Look For

#### 1. **Linear Scaling**
Operation throughput should scale linearly with batch size:
- 10 ops → X ops/sec
- 100 ops → ~10X ops/sec
- 1000 ops → ~100X ops/sec

#### 2. **Causal Delivery Overhead**
Compare these three scenarios:
- In-order: baseline (fastest)
- Random: realistic (moderate)
- Reversed: worst-case (slowest but should still be reasonable)

#### 3. **Memory Growth**
Monitor memory usage across different scales:
- 1K tracked ops → acceptable
- 100K tracked ops → manageable
- 1M tracked ops → consider GC tuning

#### 4. **Decomposition Trade-off**
Small overhead per node change but enables:
- True CRDT commutativity
- Per-node conflict resolution
- Granular operation tracking

## Optimization Tips

### If Operation Throughput is Low
1. Check batch size - larger batches should be faster per-operation
2. Review idempotency tracker implementation
3. Profile with `cargo flamegraph`

### If Causal Delivery is Slow
1. Verify operations are arriving in reasonable order
2. Check network latency and reordering
3. Consider buffering strategies
4. Review buffer size limits

### If Memory Usage is High
1. Implement periodic GC for idempotency tracker
2. Reduce max buffer size for causal delivery
3. Use persistent storage for long-term tracking
4. Monitor operation log compaction

### If End-to-End Latency is High
1. Profile each component individually
2. Check for serialization bottlenecks
3. Review network communication overhead
4. Optimize hot paths with `inline` attributes

## Continuous Benchmarking

### In CI/CD
```bash
# Run benchmarks and compare against main
cargo bench --bench replication_performance -- --save-baseline pr-branch
cargo bench --bench replication_performance -- --baseline main

# Generate comparison report
criterion-compare main pr-branch
```

### Pre-Commit
```bash
# Quick smoke test (reduced samples)
cargo bench --bench replication_performance -- --quick
```

## Related Documentation

- [Vector Clock Implementation](../src/vector_clock.rs)
- [Causal Delivery Buffer](../src/causal_delivery.rs)
- [Idempotency Tracker](../src/replay.rs)
- [Operation Decomposition](../src/operation_decomposer.rs)

## Contributing

When adding new benchmarks:
1. Follow the existing structure
2. Use realistic data sizes
3. Include both best-case and worst-case scenarios
4. Add documentation for interpretation
5. Update performance targets in this README
