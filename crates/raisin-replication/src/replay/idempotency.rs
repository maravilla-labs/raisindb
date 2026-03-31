//! Idempotency tracking for the replay engine
//!
//! Provides trait and implementations for tracking which operations
//! have already been applied to prevent duplicate application.

use std::collections::HashSet;
use std::time::Instant;
use uuid::Uuid;

use crate::metrics::{AtomicCounter, DurationHistogram, IdempotencyMetrics};

/// Trait for tracking which operations have been applied (idempotency)
///
/// This abstraction allows different implementations:
/// - In-memory HashSet (for testing, backwards compatibility)
/// - Persistent RocksDB storage (for production, survives restarts)
pub trait IdempotencyTracker: Send + Sync {
    /// Check if an operation has been applied
    fn is_applied(&self, op_id: &Uuid) -> Result<bool, String>;

    /// Mark an operation as applied
    fn mark_applied(&mut self, op_id: &Uuid, timestamp_ms: u64) -> Result<(), String>;

    /// Mark multiple operations as applied (batch operation)
    fn mark_applied_batch(&mut self, op_ids: &[(Uuid, u64)]) -> Result<(), String> {
        for (op_id, timestamp_ms) in op_ids {
            self.mark_applied(op_id, *timestamp_ms)?;
        }
        Ok(())
    }
}

/// In-memory idempotency tracker (for testing and backwards compatibility)
pub struct InMemoryIdempotencyTracker {
    applied_ops: HashSet<Uuid>,
    metrics: IdempotencyTrackerMetrics,
}

/// Metrics for idempotency tracker
#[derive(Debug, Clone)]
struct IdempotencyTrackerMetrics {
    checks_total: AtomicCounter,
    hits_total: AtomicCounter,
    misses_total: AtomicCounter,
    check_duration: DurationHistogram,
    mark_duration: DurationHistogram,
    batch_sizes: DurationHistogram, // Repurposed to track batch sizes
}

impl Default for InMemoryIdempotencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryIdempotencyTracker {
    pub fn new() -> Self {
        Self {
            applied_ops: HashSet::new(),
            metrics: IdempotencyTrackerMetrics {
                checks_total: AtomicCounter::new(),
                hits_total: AtomicCounter::new(),
                misses_total: AtomicCounter::new(),
                check_duration: DurationHistogram::new(1000),
                mark_duration: DurationHistogram::new(1000),
                batch_sizes: DurationHistogram::new(500),
            },
        }
    }

    pub fn with_applied_ops(applied_ops: HashSet<Uuid>) -> Self {
        Self {
            applied_ops,
            metrics: IdempotencyTrackerMetrics {
                checks_total: AtomicCounter::new(),
                hits_total: AtomicCounter::new(),
                misses_total: AtomicCounter::new(),
                check_duration: DurationHistogram::new(1000),
                mark_duration: DurationHistogram::new(1000),
                batch_sizes: DurationHistogram::new(500),
            },
        }
    }

    /// Get metrics for this idempotency tracker
    pub fn get_metrics(&self) -> IdempotencyMetrics {
        let checks = self.metrics.checks_total.get();
        let hits = self.metrics.hits_total.get();
        let hit_rate = if checks > 0 {
            (hits as f64 / checks as f64) * 100.0
        } else {
            0.0
        };

        // Estimate memory usage: UUID (16 bytes) + HashSet overhead (~24 bytes per entry)
        let memory_bytes = self.applied_ops.len() as u64 * 40;

        IdempotencyMetrics {
            checks_total: checks,
            hits_total: hits,
            misses_total: self.metrics.misses_total.get(),
            hit_rate_percent: hit_rate,
            tracked_operations: self.applied_ops.len() as u64,
            memory_bytes,
            disk_bytes: 0, // In-memory tracker doesn't use disk
            avg_check_duration_ms: self.metrics.check_duration.avg_ms(),
            avg_mark_duration_ms: self.metrics.mark_duration.avg_ms(),
            avg_batch_size: self.metrics.batch_sizes.avg_ms(), // Repurposed for batch sizes
            p99_check_latency_ms: self.metrics.check_duration.percentile(99.0).as_millis() as u64,
            timestamp: crate::metrics::current_timestamp_ms(),
        }
    }
}

impl IdempotencyTracker for InMemoryIdempotencyTracker {
    fn is_applied(&self, op_id: &Uuid) -> Result<bool, String> {
        let start = Instant::now();
        self.metrics.checks_total.increment();

        let result = self.applied_ops.contains(op_id);

        if result {
            self.metrics.hits_total.increment();
        } else {
            self.metrics.misses_total.increment();
        }

        self.metrics.check_duration.record(start.elapsed());
        Ok(result)
    }

    fn mark_applied(&mut self, op_id: &Uuid, _timestamp_ms: u64) -> Result<(), String> {
        let start = Instant::now();
        self.applied_ops.insert(*op_id);
        self.metrics.mark_duration.record(start.elapsed());
        Ok(())
    }

    fn mark_applied_batch(&mut self, op_ids: &[(Uuid, u64)]) -> Result<(), String> {
        let start = Instant::now();
        let batch_size = op_ids.len();

        for (op_id, _) in op_ids {
            self.applied_ops.insert(*op_id);
        }

        self.metrics.mark_duration.record(start.elapsed());
        self.metrics
            .batch_sizes
            .record(std::time::Duration::from_millis(batch_size as u64));
        Ok(())
    }
}
