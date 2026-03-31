//! Metrics and observability for CRDT replication system
//!
//! This module provides comprehensive metrics for monitoring the health and performance
//! of the distributed replication system. Metrics are collected using atomic counters
//! to minimize overhead (<1%) and exposed through structured types for easy integration
//! with monitoring systems.
//!
//! ## Metric Categories
//!
//! 1. **Causal Delivery** - Buffer state, delivery lag, ordering violations
//! 2. **Idempotency** - Hit rates, duplicate detection, storage usage
//! 3. **Operation Decomposition** - Batching efficiency, decomposition ratios
//! 4. **Replication** - Sync cycles, peer health, operation throughput
//!
//! ## Usage
//!
//! ```rust,ignore
//! // Get metrics from a component
//! let metrics = causal_buffer.get_metrics();
//! println!("Buffer utilization: {:.1}%", metrics.utilization_percent);
//!
//! // Enable periodic metrics logging
//! MetricsReporter::start(coordinator, Duration::from_secs(30));
//! ```

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Atomic counter for metrics (thread-safe, low overhead)
#[derive(Debug)]
pub struct AtomicCounter {
    value: AtomicU64,
}

impl AtomicCounter {
    pub fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    pub fn with_value(initial: u64) -> Self {
        Self {
            value: AtomicU64::new(initial),
        }
    }

    pub fn increment(&self) -> u64 {
        self.value.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn add(&self, delta: u64) -> u64 {
        self.value.fetch_add(delta, Ordering::Relaxed) + delta
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }
}

impl Default for AtomicCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AtomicCounter {
    fn clone(&self) -> Self {
        Self::with_value(self.get())
    }
}

/// Atomic gauge for size/utilization metrics
#[derive(Debug)]
pub struct AtomicGauge {
    value: AtomicUsize,
}

impl AtomicGauge {
    pub fn new() -> Self {
        Self {
            value: AtomicUsize::new(0),
        }
    }

    pub fn with_value(initial: usize) -> Self {
        Self {
            value: AtomicUsize::new(initial),
        }
    }

    pub fn set(&self, value: usize) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> usize {
        self.value.load(Ordering::Relaxed)
    }

    pub fn increment(&self) -> usize {
        self.value.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn decrement(&self) -> usize {
        self.value.fetch_sub(1, Ordering::Relaxed).saturating_sub(1)
    }

    pub fn add(&self, delta: usize) -> usize {
        self.value.fetch_add(delta, Ordering::Relaxed) + delta
    }

    pub fn sub(&self, delta: usize) -> usize {
        self.value
            .fetch_sub(delta, Ordering::Relaxed)
            .saturating_sub(delta)
    }
}

impl Default for AtomicGauge {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AtomicGauge {
    fn clone(&self) -> Self {
        Self::with_value(self.get())
    }
}

/// Histogram for tracking duration distributions
///
/// Uses a simple reservoir sampling approach to track percentiles
/// without unbounded memory growth.
#[derive(Debug)]
pub struct DurationHistogram {
    samples: Arc<std::sync::Mutex<Vec<Duration>>>,
    max_samples: usize,
    total_count: AtomicCounter,
    total_duration_ms: AtomicCounter,
}

impl DurationHistogram {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Arc::new(std::sync::Mutex::new(Vec::with_capacity(max_samples))),
            max_samples,
            total_count: AtomicCounter::new(),
            total_duration_ms: AtomicCounter::new(),
        }
    }

    pub fn record(&self, duration: Duration) {
        self.total_count.increment();
        self.total_duration_ms.add(duration.as_millis() as u64);

        let mut samples = self.samples.lock().unwrap();
        if samples.len() < self.max_samples {
            samples.push(duration);
        } else {
            // Reservoir sampling: randomly replace an existing sample
            let idx = rand::random::<usize>() % samples.len();
            samples[idx] = duration;
        }
    }

    pub fn avg_ms(&self) -> f64 {
        let count = self.total_count.get();
        if count == 0 {
            return 0.0;
        }
        self.total_duration_ms.get() as f64 / count as f64
    }

    pub fn percentile(&self, p: f64) -> Duration {
        let samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            return Duration::from_secs(0);
        }

        let mut sorted: Vec<Duration> = samples.clone();
        sorted.sort();

        let idx = ((p / 100.0) * (sorted.len() as f64)) as usize;
        let idx = idx.min(sorted.len() - 1);
        sorted[idx]
    }

    pub fn count(&self) -> u64 {
        self.total_count.get()
    }
}

impl Clone for DurationHistogram {
    fn clone(&self) -> Self {
        let samples = self.samples.lock().unwrap();
        Self {
            samples: Arc::new(std::sync::Mutex::new(samples.clone())),
            max_samples: self.max_samples,
            total_count: self.total_count.clone(),
            total_duration_ms: self.total_duration_ms.clone(),
        }
    }
}

/// Metrics for causal delivery buffer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalBufferMetrics {
    /// Current number of buffered operations
    pub current_size: usize,

    /// Maximum buffer size configured
    pub max_size: usize,

    /// Buffer utilization percentage (0-100)
    pub utilization_percent: f64,

    /// Total operations delivered since startup
    pub operations_delivered: u64,

    /// Total operations buffered (waiting for dependencies)
    pub operations_buffered: u64,

    /// Average time operations spend in buffer (ms)
    pub avg_delivery_lag_ms: f64,

    /// Age of oldest buffered operation (ms)
    pub oldest_op_age_ms: u64,

    /// Number of operations waiting on missing dependencies
    pub missing_dependencies: usize,

    /// Number of times buffer hit capacity limit
    pub buffer_full_events: u64,

    /// Operations delivered directly (no buffering)
    pub direct_deliveries: u64,

    /// p50 delivery lag (ms)
    pub p50_delivery_lag_ms: u64,

    /// p99 delivery lag (ms)
    pub p99_delivery_lag_ms: u64,

    /// Timestamp of metrics collection
    pub timestamp: u64,
}

/// Metrics for idempotency tracker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyMetrics {
    /// Total is_applied checks
    pub checks_total: u64,

    /// Operations already applied (duplicates detected)
    pub hits_total: u64,

    /// New operations (not previously applied)
    pub misses_total: u64,

    /// Hit rate percentage (0-100)
    pub hit_rate_percent: f64,

    /// Total operations tracked
    pub tracked_operations: u64,

    /// Memory usage estimate (bytes) - for in-memory tracker
    pub memory_bytes: u64,

    /// Disk usage estimate (bytes) - for persistent tracker
    pub disk_bytes: u64,

    /// Average check duration (ms)
    pub avg_check_duration_ms: f64,

    /// Average mark duration (ms)
    pub avg_mark_duration_ms: f64,

    /// Average batch size for batch operations
    pub avg_batch_size: f64,

    /// p99 check latency (ms)
    pub p99_check_latency_ms: u64,

    /// Timestamp of metrics collection
    pub timestamp: u64,
}

/// Metrics for operation decomposer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionMetrics {
    /// Original operations received
    pub operations_in: u64,

    /// Total decomposed operations produced
    pub operations_out: u64,

    /// Average expansion ratio (out/in)
    pub expansion_ratio: f64,

    /// Average decomposition duration (ms)
    pub avg_duration_ms: f64,

    /// ApplyRevision operations decomposed
    pub apply_revision_count: u64,

    /// UpsertNodeSnapshot operations produced
    pub upsert_snapshot_count: u64,

    /// DeleteNodeSnapshot operations produced
    pub delete_snapshot_count: u64,

    /// Operations passed through unchanged
    pub passthrough_count: u64,

    /// p99 decomposition latency (ms)
    pub p99_duration_ms: u64,

    /// Timestamp of metrics collection
    pub timestamp: u64,
}

/// Metrics for replication coordinator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationMetrics {
    /// Operations sent to peers (pushed)
    pub operations_pushed: u64,

    /// Operations received from peers
    pub operations_received: u64,

    /// Operations successfully applied
    pub operations_applied: u64,

    /// Operations that failed to apply
    pub operations_failed: u64,

    /// Total sync cycles executed
    pub sync_cycles: u64,

    /// Average sync duration (ms)
    pub avg_sync_duration_ms: f64,

    /// Operations behind most advanced peer
    pub replication_lag_ops: u64,

    /// Catch-up events triggered
    pub catch_up_triggered: u64,

    /// Number of currently connected peers
    pub active_peers: usize,

    /// Total configured peers
    pub total_peers: usize,

    /// CRDT conflicts detected
    pub conflicts_detected: u64,

    /// Operations skipped (already applied)
    pub operations_skipped: u64,

    /// p99 sync latency (ms)
    pub p99_sync_latency_ms: u64,

    /// Timestamp of metrics collection
    pub timestamp: u64,
}

/// Aggregate metrics for entire replication system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateMetrics {
    /// Causal delivery buffer metrics
    pub causal_buffer: CausalBufferMetrics,

    /// Idempotency tracker metrics
    pub idempotency: IdempotencyMetrics,

    /// Operation decomposition metrics
    pub decomposition: DecompositionMetrics,

    /// Replication coordinator metrics
    pub replication: ReplicationMetrics,

    /// System uptime (seconds)
    pub uptime_seconds: u64,

    /// Timestamp of metrics collection
    pub timestamp: u64,
}

/// Get current timestamp in milliseconds
pub fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as u64
}

/// Format duration as human-readable string
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else {
        format!("{:.1}h", ms as f64 / 3_600_000.0)
    }
}

/// Format byte count as human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes < KB {
        format!("{}B", bytes)
    } else if bytes < MB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_counter() {
        let counter = AtomicCounter::new();
        assert_eq!(counter.get(), 0);

        counter.increment();
        assert_eq!(counter.get(), 1);

        counter.add(5);
        assert_eq!(counter.get(), 6);
    }

    #[test]
    fn test_atomic_gauge() {
        let gauge = AtomicGauge::new();
        assert_eq!(gauge.get(), 0);

        gauge.increment();
        assert_eq!(gauge.get(), 1);

        gauge.set(10);
        assert_eq!(gauge.get(), 10);

        gauge.decrement();
        assert_eq!(gauge.get(), 9);
    }

    #[test]
    fn test_duration_histogram() {
        let hist = DurationHistogram::new(100);

        hist.record(Duration::from_millis(10));
        hist.record(Duration::from_millis(20));
        hist.record(Duration::from_millis(30));

        assert_eq!(hist.count(), 3);
        assert_eq!(hist.avg_ms(), 20.0);

        let p50 = hist.percentile(50.0);
        assert!(p50.as_millis() >= 10 && p50.as_millis() <= 30);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(1500), "1.5s");
        assert_eq!(format_duration(65_000), "1.1m");
        assert_eq!(format_duration(3_700_000), "1.0h");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(1536), "1.5KB");
        assert_eq!(format_bytes(1_572_864), "1.5MB");
        assert_eq!(format_bytes(1_610_612_736), "1.5GB");
    }
}
