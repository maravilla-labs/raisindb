//! Causal Delivery Buffer for Operation-Based CRDTs
//!
//! This module implements a causal delivery buffer that ensures operations
//! are delivered to the replay engine only when all their causal dependencies
//! have been satisfied.
//!
//! ## Why This Is Critical
//!
//! Operation-based CRDTs REQUIRE causal delivery to guarantee convergence.
//! Without it, operations could be applied before their dependencies, leading
//! to state divergence that never resolves.
//!
//! ## Example Problem This Solves
//!
//! ```ignore
//! // Node 1 creates operations:
//! Op A: CreateNode { node_id: "foo" }     VC: {node1: 1}
//! Op B: SetProperty { node_id: "foo" }    VC: {node1: 2}
//!
//! // Node 2 receives B before A (network delay)
//! // Without causal delivery:
//! //   - Apply B: FAILS (node "foo" doesn't exist)
//! //   - Apply A: Creates node, but property was never set
//! //   - RESULT: State divergence!
//! //
//! // With causal delivery:
//! //   - Receive B: Buffer it (waiting for VC {node1: 2} dependencies)
//! //   - Receive A: Apply it, then check buffer
//! //   - Now B's dependencies are satisfied: Apply B
//! //   - RESULT: Correct convergent state!
//! ```

mod delivery_logic;
#[cfg(test)]
mod tests;
pub mod types;

pub use types::BufferStats;

use hashbrown::HashMap;
use hashbrown::HashSet;
use std::time::Instant;
use uuid::Uuid;

use crate::{Operation, VectorClock};

use types::BufferMetrics;

/// Causal delivery buffer that holds operations until dependencies are satisfied
///
/// This implements the causal delivery guarantee required by operation-based CRDTs.
pub struct CausalDeliveryBuffer {
    /// Operations waiting for dependencies to be satisfied
    /// Key: operation ID, Value: (operation, set of cluster nodes this op depends on, buffered_at timestamp)
    pub(super) buffered_ops: HashMap<Uuid, (Operation, HashSet<String>, Instant)>,

    /// Local vector clock representing what this node has delivered
    /// This is updated as operations are delivered
    pub(super) local_vc: VectorClock,

    /// Maximum number of operations to buffer before warning
    /// This prevents memory exhaustion from network issues
    pub(super) max_buffer_size: usize,

    /// Statistics for monitoring
    pub(super) stats: BufferStats,

    /// Atomic metrics (thread-safe, low overhead)
    pub(in crate::causal_delivery) metrics: BufferMetrics,

    /// Timestamp when buffer was created
    created_at: Instant,
}

impl CausalDeliveryBuffer {
    /// Create a new causal delivery buffer
    ///
    /// # Arguments
    /// * `initial_vc` - The initial vector clock (typically from persistent storage)
    /// * `max_buffer_size` - Maximum operations to buffer (default: 1,000)
    pub fn new(initial_vc: VectorClock, max_buffer_size: Option<usize>) -> Self {
        Self {
            buffered_ops: HashMap::new(),
            local_vc: initial_vc,
            max_buffer_size: max_buffer_size.unwrap_or(1_000),
            stats: BufferStats::default(),
            metrics: BufferMetrics::new(),
            created_at: Instant::now(),
        }
    }

    /// Attempt to deliver an operation
    ///
    /// Returns a list of operations that can now be delivered in causal order.
    /// This includes the input operation (if deliverable) plus any buffered
    /// operations whose dependencies are now satisfied.
    pub fn deliver(&mut self, op: Operation) -> Vec<Operation> {
        // Check if operation's dependencies are satisfied
        if self.dependencies_satisfied(&op) {
            // Can deliver immediately
            self.stats.direct_deliveries += 1;
            self.stats.total_delivered += 1;
            self.metrics.direct_deliveries.increment();
            self.metrics.total_delivered.increment();

            // Record zero lag for direct delivery
            self.metrics
                .delivery_lag
                .record(std::time::Duration::from_millis(0));

            // Update local vector clock
            self.update_local_clock(&op);

            // Deliver this operation plus any buffered ops that are now ready
            let mut deliverable = vec![op];
            deliverable.extend(self.check_buffered_ops());

            deliverable
        } else {
            // Dependencies not satisfied - buffer the operation
            self.buffer_operation(op);

            // Check if any buffered operations can now be delivered
            self.check_buffered_ops()
        }
    }

    /// Get the current local vector clock
    pub fn local_vector_clock(&self) -> &VectorClock {
        &self.local_vc
    }

    /// Get buffer statistics
    pub fn stats(&self) -> &BufferStats {
        &self.stats
    }

    /// Get current buffer size
    pub fn buffer_size(&self) -> usize {
        self.buffered_ops.len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffered_ops.is_empty()
    }

    /// Clear all buffered operations (use with caution!)
    ///
    /// This should only be used in recovery scenarios where you need
    /// to re-sync from scratch.
    pub fn clear(&mut self) {
        tracing::warn!(
            buffered_ops = self.buffered_ops.len(),
            "Clearing causal delivery buffer - operations will be lost!"
        );
        self.buffered_ops.clear();
        self.stats.current_buffered = 0;
    }

    /// Get operations that have been waiting longest
    ///
    /// Useful for debugging stuck operations
    pub fn get_oldest_buffered(&self, limit: usize) -> Vec<&Operation> {
        let mut ops: Vec<&Operation> = self.buffered_ops.values().map(|(op, _, _)| op).collect();

        ops.sort_by_key(|op| op.timestamp_ms);
        ops.into_iter().take(limit).collect()
    }
}
