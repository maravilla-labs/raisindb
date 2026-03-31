# RocksDB Storage Benchmarks

Comprehensive performance benchmarks for the RocksDB storage backend.

## Quick Start

```bash
# Run quick benchmarks (20, 50, 100 nodes, 10 samples)
cargo bench --package raisin-rocksdb

# Run with specific benchmark
cargo bench --package raisin-rocksdb -- flat_create

# Run full performance suite (100, 500, 1000, 5000 nodes, 100 samples)
# First, edit benches/rocksdb_benchmarks.rs:
#   - Change BENCH_SIZES to FULL_BENCH_SIZES
#   - Change sample_size(10) to sample_size(100)
cargo bench --package raisin-rocksdb
```

## Understanding the Results

### Reading Benchmark Output

Example output:
```
flat_create/100         time:   [24.347 ms 24.594 ms 24.919 ms]
                        thrpt:  [4.0129 Kelem/s 4.0661 Kelem/s 4.1073 Kelem/s]
```

**Breaking it down:**

- **Test**: `flat_create/100` = Creating 100 nodes in a flat structure
- **Time**: `24.594 ms` (median) = Total time to create 100 nodes
- **Throughput**: `4.0661 Kelem/s` = **4,066 nodes per second**
  - `K` = Kilo (thousands)
  - `elem` = elements (nodes)
  - `/s` = per second

**Per-node calculation:**
- Time per node: 24.594ms ÷ 100 = 0.246ms per node
- Verification: 100 ÷ 0.024594s = 4,066 nodes/sec ✓

### Performance Targets

| Operation | Target | Actual (100 nodes) | Status |
|-----------|--------|-------------------|--------|
| **Node Creation** | 860/sec | ~4,000-4,700/sec | ✅ **5x faster** |
| Node Deletion | - | ~1,600/sec | ✅ |
| Node Reordering | - | ~1,100/sec | ✅ |
| List/Read Nodes | - | ~111,000/sec | ✅ **Very fast** |
| Branch Creation | - | ~1,800/sec | ✅ |

## Benchmark Suite

### Flat Structure Tests
Tests with all nodes at root level (flat hierarchy).

- **`flat_create`**: Measures throughput of creating N nodes at root
- **`flat_reorder`**: Time to reorder a single node among N siblings
- **`flat_delete`**: Time to delete a single node from N nodes
- **`flat_branch_create`**: Time to create a new branch with N existing nodes
- **`flat_list_root`**: Time to list all N root nodes

### Binary Tree Tests
Tests with nodes in a balanced binary tree structure (~log2(N) depth).

- **`tree_create`**: Measures throughput of creating N nodes in a binary tree
- **`tree_reorder`**: Time to reorder a child node in the tree
- **`tree_delete`**: Time to delete a leaf node from the tree
- **`tree_branch_create`**: Time to create a new branch with tree structure
- **`tree_list_children`**: Time to list children at a tree level

## Interpreting Results

### Creation Throughput
```
flat_create/100    thrpt: 4.0661 Kelem/s  →  4,066 nodes/second
tree_create/100    thrpt: 4.6043 Kelem/s  →  4,604 nodes/second
```
**Meaning**: Creating 100 nodes takes ~24ms (flat) or ~22ms (tree)

### Single Operation Latency
```
flat_reorder/100   thrpt: 1.0943 Kelem/s  →  1,094 operations/second
flat_reorder/100   time:  913.83 µs       →  ~0.91ms per reorder
```
**Meaning**: Each reorder operation takes ~0.91 milliseconds

### Read Performance
```
flat_list_root/100  thrpt: 111.63 Kelem/s  →  111,630 nodes/second
```
**Meaning**: Can read ~112,000 nodes per second (extremely fast!)

## Sample Results

Recent benchmark run (2025-01-20):

```
Flat Structure (100 nodes):
  Create:   4,066 nodes/sec  (24.6ms total, 0.246ms/node)
  Reorder:  1,094 ops/sec    (0.91ms per operation)
  Delete:   1,627 ops/sec    (0.61ms per operation)
  List:     111,630 nodes/sec (0.89ms to list 100 nodes)
  Branch:   1,801 ops/sec    (0.56ms per operation)

Binary Tree (100 nodes):
  Create:   4,604 nodes/sec  (21.7ms total, 0.217ms/node)
  Reorder:  1,603 ops/sec    (0.62ms per operation)
  Delete:   1,683 ops/sec    (0.59ms per operation)
  List:     3,309 nodes/sec  (0.60ms to list 2 children)
  Branch:   1,777 ops/sec    (0.56ms per operation)
```

## Configuration

### Quick Test (Default)
- **Node counts**: 20, 50, 100
- **Samples**: 10 iterations
- **Duration**: ~2-3 minutes
- **Purpose**: Quick validation, CI/CD

### Full Performance Test
- **Node counts**: 100, 500, 1000, 5000
- **Samples**: 100 iterations
- **Duration**: ~30-60 minutes
- **Purpose**: Comprehensive performance analysis

### Adjusting Benchmark Size

Edit `crates/raisin-rocksdb/benches/rocksdb_benchmarks.rs`:

```rust
// For quick tests (default):
const BENCH_SIZES: &[usize] = &[20, 50, 100];

// For full performance analysis (uncomment):
// const BENCH_SIZES: &[usize] = &[100, 500, 1000, 5000];
```

Also update `sample_size(10)` to `sample_size(100)` in each benchmark function.

## Output Files

Criterion generates detailed reports in:
- `target/criterion/flat_create/` - HTML reports with graphs
- `target/criterion/flat_create/report/index.html` - View in browser
- Raw data in CSV format for further analysis

## Troubleshooting

### Benchmarks Taking Too Long
- Reduce `BENCH_SIZES` to smaller values (e.g., `&[10, 20, 50]`)
- Reduce `sample_size` from 10 to 5
- Run specific benchmarks: `cargo bench -- flat_create/20`

### Memory Issues
- RocksDB uses significant memory for larger datasets
- Each benchmark creates a fresh database
- Consider reducing max node count for constrained environments

## Technical Details

### Test Setup
Each benchmark:
1. Creates a temporary RocksDB database
2. Initializes tenant, repository, branch, and workspace
3. Runs the operation multiple times (sample_size)
4. Cleans up temporary database
5. Reports median time and throughput

### Isolation
- Each iteration gets a fresh database (for operations that modify data)
- Read-only operations reuse the same database across iterations
- No cross-contamination between benchmarks

### Measurement
- Uses Criterion.rs for statistical analysis
- Reports confidence intervals (lower/median/upper bounds)
- Detects performance regressions across runs
- Identifies outliers automatically

---

## Persistent Idempotency Tracker Benchmarks

### Overview

The `persistent_idempotency_bench.rs` benchmark compares in-memory vs persistent (RocksDB-backed) idempotency tracking for CRDT replication.

#### Why This Matters

The idempotency tracker is critical for CRDT correctness:
- **Prevents duplicate application** of operations across restarts
- **Survives crashes** when persisted to RocksDB
- **Performance trade-off** between memory and durability

### Running the Benchmark

```bash
# Run all idempotency benchmarks
cargo bench --bench persistent_idempotency_bench

# Run specific groups
cargo bench --bench persistent_idempotency_bench -- comparison
cargo bench --bench persistent_idempotency_bench -- persistent
cargo bench --bench persistent_idempotency_bench -- realistic
```

### Benchmark Groups

#### Comparison Benchmarks
Direct comparison of in-memory vs persistent implementations:

- `is_applied_comparison` - Lookup performance (1K, 10K, 100K tracked ops)
- `mark_applied_comparison` - Single operation marking
- `batch_mark_applied_comparison` - Batch marking (10, 100, 1K ops)

#### Persistent-Only Benchmarks
Features specific to RocksDB storage:

- `persistent_load_all` - Load all tracked ops into memory
- `persistent_count` - Count tracked operations
- `persistent_gc` - Garbage collection of old operations

#### Realistic Workloads
Real-world usage patterns:

- `realistic_catch_up` - Catch-up scenario (check + mark)
- `realistic_normal_operation` - Normal operation (90% hits, 10% misses)

### Expected Performance

Based on typical hardware (SSD, 8-core CPU):

#### In-Memory Tracker
- **Lookup**: 10-20 ns
- **Mark single**: 50-100 ns
- **Mark batch (100)**: 5-10 µs
- **Memory**: ~24 bytes/operation

#### Persistent Tracker (RocksDB)
- **Lookup**: 300-500 ns (with OS page cache)
- **Mark single**: 1-2 µs
- **Mark batch (100)**: 100-200 µs
- **Disk space**: ~50-100 bytes/operation (compressed)

**Trade-off**: Persistent is 20-30x slower but provides crash recovery.

### Usage Recommendations

#### Use In-Memory When:
- Testing and development
- Read replicas that can rebuild
- Non-critical workloads

#### Use Persistent When:
- Production deployments
- Critical data replication
- Master/primary nodes

#### Hybrid Approach (Best of Both):
```rust
// Load into memory at startup, persist in background
let persistent = PersistentIdempotencyTracker::new(db, "applied_ops");
let cached_ops = persistent.load_all_applied()?;
let in_memory = InMemoryIdempotencyTracker::with_applied_ops(cached_ops);

// Use in_memory for lookups, persistent for marking
if !in_memory.is_applied(&op_id)? {
    persistent.mark_applied(&op_id, timestamp)?;
    in_memory.mark_applied(&op_id, timestamp)?;
}
```

### Optimization Tips

1. **Use Batch Operations**
```rust
// Good: Single batch mark (fast)
let batch: Vec<(Uuid, u64)> = operations
    .into_iter()
    .map(|op| (op.op_id, op.timestamp_ms))
    .collect();
tracker.mark_applied_batch(batch.into_iter())?;
```

2. **Implement Periodic GC**
```rust
// Keep only recent operations (30 days)
let ttl = 30 * 24 * 60 * 60 * 1000;
tracker.gc_old_operations(current_time, ttl)?;
```

3. **Monitor Performance**
- In-memory lookup should be <50ns
- Persistent lookup should be <1µs (with cache)
- If slower, check RocksDB configuration
