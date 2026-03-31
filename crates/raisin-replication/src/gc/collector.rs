//! Garbage collector implementation for the operation log

use crate::operation::Operation;

use super::config::{GcConfig, GcStrategy};
use super::watermarks::PeerWatermarks;

/// Garbage collector for operation log
pub struct GarbageCollector {
    config: GcConfig,
    watermarks: PeerWatermarks,
}

impl GarbageCollector {
    /// Create a new garbage collector with default configuration
    pub fn new() -> Self {
        Self {
            config: GcConfig::default(),
            watermarks: PeerWatermarks::new(),
        }
    }

    /// Create a garbage collector with custom configuration
    pub fn with_config(config: GcConfig) -> Self {
        Self {
            config,
            watermarks: PeerWatermarks::new(),
        }
    }

    /// Create a garbage collector with existing watermarks
    pub fn with_watermarks(config: GcConfig, watermarks: PeerWatermarks) -> Self {
        Self { config, watermarks }
    }

    /// Update peer acknowledgments based on operations
    ///
    /// This scans the operation log and updates watermarks based on the
    /// `acknowledged_by` field in each operation.
    pub fn update_watermarks_from_operations(&mut self, operations: &[Operation]) {
        for op in operations {
            for peer_id in &op.acknowledged_by {
                self.watermarks.update(peer_id.clone(), op.op_seq);
            }
        }
    }

    /// Mark an operation as acknowledged by a peer
    pub fn acknowledge_operation(&mut self, node_id: String, op_seq: u64) {
        self.watermarks.update(node_id, op_seq);
    }

    /// Determine which operations can be safely deleted
    ///
    /// This is the main GC decision engine. It returns a list of operation IDs
    /// that are safe to delete based on the current strategy.
    pub fn collect(
        &self,
        operations: &[Operation],
        current_log_size_bytes: u64,
    ) -> (Vec<uuid::Uuid>, GcStrategy) {
        // Emergency GC takes precedence
        if self.config.emergency_gc_enabled
            && current_log_size_bytes > self.config.max_log_size_bytes
        {
            return self.collect_emergency(operations);
        }

        // Try acknowledgment-based GC first
        let (to_delete, _strategy) = self.collect_acknowledgment_based(operations);

        // If acknowledgment-based GC found operations to delete, use it
        if !to_delete.is_empty() {
            return (to_delete, GcStrategy::AcknowledgmentBased);
        }

        // Fall back to time-based GC as a fail-safe
        // This ensures old operations are eventually deleted even if peers never acknowledge
        self.collect_time_based(operations)
    }

    /// Determine which GC strategy to use
    fn determine_strategy(&self, current_log_size_bytes: u64) -> GcStrategy {
        // Emergency takes precedence
        if self.config.emergency_gc_enabled
            && current_log_size_bytes > self.config.max_log_size_bytes
        {
            return GcStrategy::Emergency;
        }

        // Time-based fail-safe is always active as a safety net
        GcStrategy::AcknowledgmentBased
    }

    /// Collect operations using acknowledgment-based strategy
    ///
    /// Delete operations that have been acknowledged by all known peers
    /// (or min_peer_acknowledgments if configured).
    fn collect_acknowledgment_based(
        &self,
        operations: &[Operation],
    ) -> (Vec<uuid::Uuid>, GcStrategy) {
        let min_watermark = self.watermarks.min_watermark();
        let mut to_delete = Vec::new();

        for op in operations {
            // Check if enough peers have acknowledged this operation
            let ack_count = op.acknowledged_by.len();
            let peer_count = self.watermarks.peers().len();

            let can_delete = if self.config.min_peer_acknowledgments == 0 {
                // Wait for all peers
                ack_count >= peer_count && peer_count > 0 && op.op_seq <= min_watermark
            } else {
                // Wait for minimum number of peers
                ack_count >= self.config.min_peer_acknowledgments && op.op_seq <= min_watermark
            };

            if can_delete {
                to_delete.push(op.op_id);
            }
        }

        (to_delete, GcStrategy::AcknowledgmentBased)
    }

    /// Collect operations using time-based fail-safe strategy
    ///
    /// Force delete operations older than max_age_days, regardless of
    /// peer acknowledgments. This prevents permanently offline peers
    /// from blocking GC indefinitely.
    fn collect_time_based(&self, operations: &[Operation]) -> (Vec<uuid::Uuid>, GcStrategy) {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let cutoff_ms = now_ms - (self.config.max_age_days * 24 * 60 * 60 * 1000);
        let mut to_delete = Vec::new();

        for op in operations {
            if op.timestamp_ms < cutoff_ms {
                to_delete.push(op.op_id);
            }
        }

        (to_delete, GcStrategy::TimeBasedFailsafe)
    }

    /// Collect operations using emergency strategy
    ///
    /// Aggressively delete operations to bring log size back to target.
    /// Uses time-based deletion, starting with oldest operations first.
    fn collect_emergency(&self, operations: &[Operation]) -> (Vec<uuid::Uuid>, GcStrategy) {
        // Sort operations by timestamp (oldest first)
        let mut sorted_ops = operations.to_vec();
        sorted_ops.sort_by_key(|op| op.timestamp_ms);

        let mut to_delete = Vec::new();
        let avg_op_size = if operations.is_empty() {
            1024 // Assume 1KB per operation
        } else {
            self.config.max_log_size_bytes / operations.len() as u64
        };

        let bytes_to_reclaim = self.config.max_log_size_bytes - self.config.target_log_size_bytes;
        let ops_to_delete = (bytes_to_reclaim / avg_op_size) as usize;

        for op in sorted_ops.iter().take(ops_to_delete) {
            to_delete.push(op.op_id);
        }

        (to_delete, GcStrategy::Emergency)
    }

    /// Get the current watermarks
    pub fn watermarks(&self) -> &PeerWatermarks {
        &self.watermarks
    }

    /// Get mutable watermarks
    pub fn watermarks_mut(&mut self) -> &mut PeerWatermarks {
        &mut self.watermarks
    }

    /// Get the GC configuration
    pub fn config(&self) -> &GcConfig {
        &self.config
    }

    /// Update the GC configuration
    pub fn update_config(&mut self, config: GcConfig) {
        self.config = config;
    }

    /// Remove a peer from watermarks (when permanently offline)
    pub fn remove_peer(&mut self, node_id: &str) {
        self.watermarks.remove_peer(node_id);
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}
