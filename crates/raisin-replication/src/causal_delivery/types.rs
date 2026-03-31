//! Types for the causal delivery buffer

use crate::metrics::{AtomicCounter, AtomicGauge, DurationHistogram};

/// Atomic metrics for causal delivery buffer
#[derive(Debug, Clone)]
pub(super) struct BufferMetrics {
    /// Total operations delivered
    pub(super) total_delivered: AtomicCounter,

    /// Total operations buffered
    pub(super) total_buffered: AtomicCounter,

    /// Direct deliveries (no buffering)
    pub(super) direct_deliveries: AtomicCounter,

    /// Buffer full events
    pub(super) buffer_full_events: AtomicCounter,

    /// Current buffer size
    pub(super) current_size: AtomicGauge,

    /// Max buffer size seen
    pub(super) max_size_seen: AtomicGauge,

    /// Delivery lag histogram
    pub(super) delivery_lag: DurationHistogram,
}

impl BufferMetrics {
    pub(super) fn new() -> Self {
        Self {
            total_delivered: AtomicCounter::new(),
            total_buffered: AtomicCounter::new(),
            direct_deliveries: AtomicCounter::new(),
            buffer_full_events: AtomicCounter::new(),
            current_size: AtomicGauge::new(),
            max_size_seen: AtomicGauge::new(),
            delivery_lag: DurationHistogram::new(1000),
        }
    }
}

/// Statistics about the causal delivery buffer
#[derive(Debug, Clone, Default)]
pub struct BufferStats {
    /// Total operations buffered since creation
    pub total_buffered: u64,

    /// Total operations delivered since creation
    pub total_delivered: u64,

    /// Current number of buffered operations
    pub current_buffered: usize,

    /// Maximum buffer size reached
    pub max_buffered: usize,

    /// Number of times dependencies were not satisfied
    pub dependency_waits: u64,

    /// Number of operations delivered directly (no buffering needed)
    pub direct_deliveries: u64,
}
