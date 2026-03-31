# Replication Metrics and Observability

This document describes the comprehensive metrics system for monitoring the distributed CRDT replication system in RaisinDB.

## Overview

The replication system exposes detailed metrics across four major components:
1. **Causal Delivery Buffer** - Ensures operations are delivered in happens-before order
2. **Idempotency Tracker** - Prevents duplicate operation application
3. **Operation Decomposer** - Breaks batched operations into atomic CRDT operations
4. **Replication Coordinator** - Manages peer synchronization and operation distribution

All metrics are collected using atomic counters with **<1% overhead** and are designed for production use in high-scale systems.

## Quick Start

```rust
use raisin_replication::{ReplicationCoordinator, MetricsReporter};
use std::time::Duration;

// Create coordinator
let coordinator = ReplicationCoordinator::new(cluster_config, storage)?;

// Start periodic metrics logging (every 30 seconds)
let reporter = MetricsReporter::new(Duration::from_secs(30));
reporter.start(|| async {
    // Collect metrics from all components
    get_aggregate_metrics(&coordinator).await
});
```

## Metric Categories

### 1. Causal Delivery Buffer Metrics

The causal buffer ensures operations are delivered in causal order, preventing state divergence.

**Key Metrics:**
- `current_size` - Operations currently buffered
- `utilization_percent` - Buffer utilization (0-100%)
- `operations_delivered` - Total ops delivered since startup
- `direct_deliveries` - Ops delivered immediately (no buffering needed)
- `avg_delivery_lag_ms` - Average time ops spend in buffer
- `oldest_op_age_ms` - Age of oldest buffered operation
- `missing_dependencies` - Ops waiting on missing dependencies
- `buffer_full_events` - Times buffer hit capacity

**Collection:**
```rust
let metrics = causal_buffer.get_metrics();
println!("Buffer utilization: {:.1}%", metrics.utilization_percent);
println!("Delivery lag: {:.1}ms avg, {} p99",
    metrics.avg_delivery_lag_ms,
    metrics.p99_delivery_lag_ms
);
```

**Alert Thresholds:**
- `utilization_percent > 80%` - Buffer filling up, possible network issues
- `oldest_op_age_ms > 5000` - Operations stuck for >5s, investigate dependencies
- `buffer_full_events > 0` - Buffer capacity reached, increase `max_buffer_size`

### 2. Idempotency Tracker Metrics

Tracks which operations have been applied to prevent duplicates across restarts.

**Key Metrics:**
- `checks_total` - Total idempotency checks performed
- `hits_total` - Duplicate operations detected
- `hit_rate_percent` - Percentage of checks that were duplicates
- `tracked_operations` - Total operations tracked
- `memory_bytes` / `disk_bytes` - Storage usage
- `avg_check_duration_ms` - Average lookup latency
- `avg_mark_duration_ms` - Average write latency

**Collection:**
```rust
// In-memory tracker
let tracker = InMemoryIdempotencyTracker::new();
let metrics = tracker.get_metrics();
println!("Hit rate: {:.1}%, Memory: {}",
    metrics.hit_rate_percent,
    format_bytes(metrics.memory_bytes)
);

// Persistent tracker (RocksDB)
let tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());
let metrics = tracker.get_metrics();
println!("Disk usage: {}, p99 latency: {}ms",
    format_bytes(metrics.disk_bytes),
    metrics.p99_check_latency_ms
);
```

**Alert Thresholds:**
- `hit_rate_percent > 50%` - High duplicate rate, investigate why operations are being replayed
- `tracked_operations > 10_000_000` - Consider running garbage collection
- `avg_check_duration_ms > 1.0` - Slow lookups, check disk/memory performance

### 3. Operation Decomposition Metrics

Tracks decomposition of batched operations into atomic CRDT operations.

**Key Metrics:**
- `operations_in` - Original operations received
- `operations_out` - Total decomposed operations produced
- `expansion_ratio` - Average expansion ratio (out/in)
- `avg_duration_ms` - Decomposition latency
- `apply_revision_count` - ApplyRevision operations decomposed
- `upsert_snapshot_count` / `delete_snapshot_count` - Decomposed operation types
- `passthrough_count` - Operations passed through unchanged

**Collection:**
```rust
let decomposer = OperationDecomposer::new();

// Decompose a single operation
let decomposed = decomposer.decompose(operation);

// Decompose a batch
let decomposed = decomposer.decompose_batch(operations);

// Get metrics
let metrics = decomposer.get_metrics();
println!("Expansion ratio: {:.1}x, avg latency: {:.2}ms",
    metrics.expansion_ratio,
    metrics.avg_duration_ms
);
```

**Alert Thresholds:**
- `expansion_ratio > 10.0` - Very large batches, consider smaller revisions
- `avg_duration_ms > 5.0` - Slow decomposition, investigate operation complexity

### 4. Replication Coordinator Metrics

Tracks overall replication health, peer synchronization, and operation throughput.

**Key Metrics:**
- `operations_pushed` / `operations_received` - Operation flow
- `operations_applied` / `operations_failed` - Application results
- `operations_skipped` - Duplicates detected during replay
- `sync_cycles` - Total sync cycles executed
- `avg_sync_duration_ms` - Average sync latency
- `active_peers` / `total_peers` - Peer connectivity
- `conflicts_detected` - CRDT conflicts encountered
- `replication_lag_ops` - Operations behind most advanced peer
- `catch_up_triggered` - Full state catch-up events

**Collection:**
```rust
let coordinator = ReplicationCoordinator::new(config, storage)?;
let metrics = coordinator.get_metrics().await;

println!("Active peers: {}/{}", metrics.active_peers, metrics.total_peers);
println!("Throughput: {} ops/cycle",
    metrics.operations_applied / metrics.sync_cycles.max(1)
);
println!("Conflicts: {}", metrics.conflicts_detected);
```

**Alert Thresholds:**
- `active_peers < total_peers` - Some peers disconnected
- `operations_failed > 0` - Operations failing to apply, investigate errors
- `replication_lag_ops > 1000` - Significant lag, may need catch-up
- `avg_sync_duration_ms > 1000` - Slow syncs, check network/storage performance

## Aggregate Metrics

Collect metrics from all components in one call:

```rust
use raisin_replication::{AggregateMetrics, metrics_to_json};

async fn get_aggregate_metrics(
    coordinator: &ReplicationCoordinator,
    causal_buffer: &CausalDeliveryBuffer,
    decomposer: &OperationDecomposer,
) -> AggregateMetrics {
    let start_time = std::time::Instant::now();

    AggregateMetrics {
        causal_buffer: causal_buffer.get_metrics(),
        idempotency: get_idempotency_metrics(), // From your tracker
        decomposition: decomposer.get_metrics(),
        replication: coordinator.get_metrics().await,
        uptime_seconds: start_time.elapsed().as_secs(),
        timestamp: current_timestamp_ms(),
    }
}

// Export as JSON
let metrics = get_aggregate_metrics(&coordinator, &buffer, &decomposer).await;
let json = metrics_to_json(&metrics)?;
println!("{}", json);
```

## Periodic Metrics Reporting

Enable automatic periodic logging:

```rust
use raisin_replication::MetricsReporter;
use std::time::Duration;

let reporter = MetricsReporter::new(Duration::from_secs(30));

// Start background reporting task
reporter.start(|| async {
    AggregateMetrics {
        causal_buffer: my_buffer.get_metrics(),
        idempotency: my_tracker.get_metrics(),
        decomposition: my_decomposer.get_metrics(),
        replication: my_coordinator.get_metrics().await,
        uptime_seconds: uptime.elapsed().as_secs(),
        timestamp: current_timestamp_ms(),
    }
});
```

**Example Output:**
```
=== Replication Metrics (uptime: 3600s) ===
--- Causal Delivery Buffer ---
  Size: 5/10000 (0.0% utilization)
  Operations: 1000 delivered (990 direct), 10 buffered
  Delivery lag: 5.2ms avg, 3ms p50, 15ms p99
--- Idempotency Tracker ---
  Checks: 1500 total, 500 hits (33.3% hit rate)
  Tracked operations: 1000 (mem: 39.1KB, disk: 0B)
  Latency: 0.05ms check avg, 0.10ms mark avg, 1ms p99
  Batch size: 10.0 operations avg
--- Operation Decomposer ---
  Operations: 100 in, 250 out (2.5x expansion)
  Breakdown: 50 ApplyRevision, 50 passthrough
  Decomposed ops: 120 upserts, 30 deletes
  Latency: 0.15ms avg, 1ms p99
--- Replication Coordinator ---
  Peers: 2/3 active
  Operations: 500 pushed, 600 received, 590 applied
  Skipped (duplicates): 10
  Sync cycles: 100 (avg: 50.0ms, p99: 150ms)
  Conflicts detected: 5
```

## Prometheus Integration

Export metrics in Prometheus format:

```rust
// Add to your HTTP server
async fn metrics_handler() -> impl warp::Reply {
    let metrics = get_aggregate_metrics().await;

    // Convert to Prometheus format
    let prometheus = format!(
        r#"
# HELP raisindb_replication_operations_total Total operations by type
# TYPE raisindb_replication_operations_total counter
raisindb_replication_operations_total{{type="pushed"}} {}
raisindb_replication_operations_total{{type="received"}} {}
raisindb_replication_operations_total{{type="applied"}} {}

# HELP raisindb_causal_buffer_size Current causal buffer size
# TYPE raisindb_causal_buffer_size gauge
raisindb_causal_buffer_size {}

# HELP raisindb_replication_lag_seconds Replication lag in seconds
# TYPE raisindb_replication_lag_seconds gauge
raisindb_replication_lag_seconds {}
"#,
        metrics.replication.operations_pushed,
        metrics.replication.operations_received,
        metrics.replication.operations_applied,
        metrics.causal_buffer.current_size,
        metrics.replication.replication_lag_ops,
    );

    warp::reply::with_header(prometheus, "content-type", "text/plain")
}
```

## Performance Impact

Metrics collection is designed for production use with minimal overhead:

- **Atomic counters**: ~5ns per increment (lock-free)
- **Histogram sampling**: Reservoir sampling (bounded memory)
- **No allocations**: Most metrics use stack-allocated atomics
- **Total overhead**: <1% CPU and <10MB memory per component

## Best Practices

1. **Monitor buffer utilization**: Keep `causal_buffer.utilization_percent < 80%`
2. **Track hit rates**: High `idempotency.hit_rate` may indicate replay issues
3. **Watch conflicts**: Frequent `conflicts_detected` suggests concurrent write patterns
4. **Alert on lag**: Set alerts when `replication_lag_ops > threshold`
5. **Periodic GC**: Run idempotency GC when `tracked_operations > 10M`
6. **Log metrics**: Enable periodic reporting for visibility

## Troubleshooting

### High Buffer Utilization
**Symptom:** `causal_buffer.utilization_percent > 80%`

**Causes:**
- Network partition between peers
- Slow peer lagging behind
- Operations arriving out of order

**Solutions:**
- Check network connectivity
- Investigate slow peers (check their metrics)
- Increase `max_buffer_size` if needed
- Consider triggering catch-up protocol

### High Idempotency Hit Rate
**Symptom:** `idempotency.hit_rate_percent > 50%`

**Causes:**
- Operations being replayed after restart
- Duplicate messages from peers
- Catch-up protocol re-delivering operations

**Solutions:**
- Ensure persistent idempotency tracking is enabled
- Check peer sync logic for duplicates
- Verify catch-up protocol is working correctly

### Replication Lag
**Symptom:** `replication.replication_lag_ops > 1000`

**Causes:**
- Slow storage backend
- Network bandwidth limitations
- High write volume

**Solutions:**
- Optimize storage performance
- Increase sync batch size
- Enable compression for network transfer
- Consider horizontal scaling

## API Reference

See inline documentation in:
- `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-replication/src/metrics.rs`
- `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-replication/src/metrics_reporter.rs`
- `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-replication/src/causal_delivery.rs` (get_metrics method)
- `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-replication/src/replay.rs` (get_metrics method)
- `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-replication/src/operation_decomposer_metrics.rs`
- `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-replication/src/coordinator.rs` (get_metrics method)
