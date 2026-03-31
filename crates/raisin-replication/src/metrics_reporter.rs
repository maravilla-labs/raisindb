//! Metrics reporting and aggregation
//!
//! This module provides periodic metrics logging and aggregation across
//! all replication components.

use crate::metrics::{
    format_bytes, format_duration, AggregateMetrics, CausalBufferMetrics, DecompositionMetrics,
    IdempotencyMetrics, ReplicationMetrics,
};
use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::{info, warn};

/// Periodic metrics reporter
///
/// This background task periodically collects and logs metrics from all
/// replication components.
pub struct MetricsReporter {
    interval_duration: Duration,
    started_at: Instant,
}

impl MetricsReporter {
    /// Create a new metrics reporter
    ///
    /// # Arguments
    /// * `interval_duration` - How often to report metrics (e.g., Duration::from_secs(30))
    pub fn new(interval_duration: Duration) -> Self {
        Self {
            interval_duration,
            started_at: Instant::now(),
        }
    }

    /// Start periodic metrics reporting
    ///
    /// This spawns a background task that collects and logs metrics at regular intervals.
    ///
    /// # Arguments
    /// * `metrics_fn` - Async function that returns AggregateMetrics
    pub fn start<F, Fut>(self, metrics_fn: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = AggregateMetrics> + Send,
    {
        let interval_duration = self.interval_duration;

        tokio::spawn(async move {
            let mut ticker = interval(interval_duration);

            loop {
                ticker.tick().await;

                let metrics = metrics_fn().await;
                Self::log_metrics(&metrics);
            }
        });
    }

    /// Log aggregate metrics in a structured format
    fn log_metrics(metrics: &AggregateMetrics) {
        info!(
            "=== Replication Metrics (uptime: {}s) ===",
            metrics.uptime_seconds
        );

        Self::log_causal_buffer_metrics(&metrics.causal_buffer);
        Self::log_idempotency_metrics(&metrics.idempotency);
        Self::log_decomposition_metrics(&metrics.decomposition);
        Self::log_replication_metrics(&metrics.replication);
    }

    /// Log causal delivery buffer metrics
    fn log_causal_buffer_metrics(metrics: &CausalBufferMetrics) {
        info!("--- Causal Delivery Buffer ---");
        info!(
            "  Size: {}/{} ({:.1}% utilization)",
            metrics.current_size, metrics.max_size, metrics.utilization_percent
        );
        info!(
            "  Operations: {} delivered ({} direct), {} buffered",
            metrics.operations_delivered, metrics.direct_deliveries, metrics.operations_buffered
        );
        info!(
            "  Delivery lag: {:.1}ms avg, {} p50, {} p99",
            metrics.avg_delivery_lag_ms,
            format_duration(metrics.p50_delivery_lag_ms),
            format_duration(metrics.p99_delivery_lag_ms)
        );

        if metrics.missing_dependencies > 0 {
            warn!(
                "  Missing dependencies: {} operations waiting, oldest: {}",
                metrics.missing_dependencies,
                format_duration(metrics.oldest_op_age_ms)
            );
        }

        if metrics.buffer_full_events > 0 {
            warn!("  Buffer full events: {}", metrics.buffer_full_events);
        }
    }

    /// Log idempotency tracker metrics
    fn log_idempotency_metrics(metrics: &IdempotencyMetrics) {
        info!("--- Idempotency Tracker ---");
        info!(
            "  Checks: {} total, {} hits ({:.1}% hit rate)",
            metrics.checks_total, metrics.hits_total, metrics.hit_rate_percent
        );
        info!(
            "  Tracked operations: {} (mem: {}, disk: {})",
            metrics.tracked_operations,
            format_bytes(metrics.memory_bytes),
            format_bytes(metrics.disk_bytes)
        );
        info!(
            "  Latency: {:.2}ms check avg, {:.2}ms mark avg, {} p99",
            metrics.avg_check_duration_ms,
            metrics.avg_mark_duration_ms,
            format_duration(metrics.p99_check_latency_ms)
        );
        if metrics.avg_batch_size > 0.0 {
            info!("  Batch size: {:.1} operations avg", metrics.avg_batch_size);
        }
    }

    /// Log operation decomposition metrics
    fn log_decomposition_metrics(metrics: &DecompositionMetrics) {
        info!("--- Operation Decomposer ---");
        info!(
            "  Operations: {} in, {} out ({}x expansion)",
            metrics.operations_in, metrics.operations_out, metrics.expansion_ratio
        );
        info!(
            "  Breakdown: {} ApplyRevision, {} passthrough",
            metrics.apply_revision_count, metrics.passthrough_count
        );
        info!(
            "  Decomposed ops: {} upserts, {} deletes",
            metrics.upsert_snapshot_count, metrics.delete_snapshot_count
        );
        info!(
            "  Latency: {:.2}ms avg, {} p99",
            metrics.avg_duration_ms,
            format_duration(metrics.p99_duration_ms)
        );
    }

    /// Log replication coordinator metrics
    fn log_replication_metrics(metrics: &ReplicationMetrics) {
        info!("--- Replication Coordinator ---");
        info!(
            "  Peers: {}/{} active",
            metrics.active_peers, metrics.total_peers
        );
        info!(
            "  Operations: {} pushed, {} received, {} applied",
            metrics.operations_pushed, metrics.operations_received, metrics.operations_applied
        );
        if metrics.operations_failed > 0 {
            warn!("  Failed operations: {}", metrics.operations_failed);
        }
        if metrics.operations_skipped > 0 {
            info!("  Skipped (duplicates): {}", metrics.operations_skipped);
        }
        info!(
            "  Sync cycles: {} (avg: {:.1}ms, p99: {})",
            metrics.sync_cycles,
            metrics.avg_sync_duration_ms,
            format_duration(metrics.p99_sync_latency_ms)
        );
        if metrics.replication_lag_ops > 0 {
            warn!(
                "  Replication lag: {} operations behind",
                metrics.replication_lag_ops
            );
        }
        if metrics.conflicts_detected > 0 {
            info!("  Conflicts detected: {}", metrics.conflicts_detected);
        }
        if metrics.catch_up_triggered > 0 {
            info!("  Catch-up events: {}", metrics.catch_up_triggered);
        }
    }
}

/// Format metrics as JSON string
///
/// Useful for exposing metrics via HTTP API or writing to files.
pub fn metrics_to_json(metrics: &AggregateMetrics) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(metrics)
}

/// Format metrics as compact JSON string (no whitespace)
pub fn metrics_to_json_compact(metrics: &AggregateMetrics) -> Result<String, serde_json::Error> {
    serde_json::to_string(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_metrics() -> AggregateMetrics {
        AggregateMetrics {
            causal_buffer: CausalBufferMetrics {
                current_size: 5,
                max_size: 10000,
                utilization_percent: 0.05,
                operations_delivered: 1000,
                operations_buffered: 10,
                avg_delivery_lag_ms: 5.2,
                oldest_op_age_ms: 100,
                missing_dependencies: 2,
                buffer_full_events: 0,
                direct_deliveries: 990,
                p50_delivery_lag_ms: 3,
                p99_delivery_lag_ms: 15,
                timestamp: 1234567890,
            },
            idempotency: IdempotencyMetrics {
                checks_total: 1500,
                hits_total: 500,
                misses_total: 1000,
                hit_rate_percent: 33.33,
                tracked_operations: 1000,
                memory_bytes: 40000,
                disk_bytes: 0,
                avg_check_duration_ms: 0.05,
                avg_mark_duration_ms: 0.1,
                avg_batch_size: 10.0,
                p99_check_latency_ms: 1,
                timestamp: 1234567890,
            },
            decomposition: DecompositionMetrics {
                operations_in: 100,
                operations_out: 250,
                expansion_ratio: 2.5,
                avg_duration_ms: 0.15,
                apply_revision_count: 50,
                upsert_snapshot_count: 120,
                delete_snapshot_count: 30,
                passthrough_count: 50,
                p99_duration_ms: 1,
                timestamp: 1234567890,
            },
            replication: ReplicationMetrics {
                operations_pushed: 500,
                operations_received: 600,
                operations_applied: 590,
                operations_failed: 1,
                sync_cycles: 100,
                avg_sync_duration_ms: 50.0,
                replication_lag_ops: 0,
                catch_up_triggered: 0,
                active_peers: 2,
                total_peers: 3,
                conflicts_detected: 5,
                operations_skipped: 10,
                p99_sync_latency_ms: 150,
                timestamp: 1234567890,
            },
            uptime_seconds: 3600,
            timestamp: 1234567890,
        }
    }

    #[test]
    fn test_metrics_to_json() {
        let metrics = make_test_metrics();
        let json = metrics_to_json(&metrics).unwrap();
        assert!(json.contains("causal_buffer"));
        assert!(json.contains("idempotency"));
        assert!(json.contains("decomposition"));
        assert!(json.contains("replication"));
    }

    #[test]
    fn test_metrics_to_json_compact() {
        let metrics = make_test_metrics();
        let json = metrics_to_json_compact(&metrics).unwrap();
        assert!(!json.contains('\n')); // No newlines in compact format
        assert!(json.contains("causal_buffer"));
    }
}
