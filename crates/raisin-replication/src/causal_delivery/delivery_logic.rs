//! Internal delivery logic for the causal delivery buffer
//!
//! Handles dependency checking, buffering, and cascading delivery.

use hashbrown::HashSet;
use std::time::Instant;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::metrics::{current_timestamp_ms, CausalBufferMetrics};
use crate::Operation;

use super::CausalDeliveryBuffer;

impl CausalDeliveryBuffer {
    /// Check if an operation's causal dependencies are satisfied
    ///
    /// An operation's dependencies are satisfied if our local vector clock
    /// is >= the operation's vector clock for all cluster nodes.
    pub(super) fn dependencies_satisfied(&self, op: &Operation) -> bool {
        // For each cluster node in the operation's vector clock,
        // check if our local clock has seen at least that many operations
        for (node_id, op_counter) in op.vector_clock.as_map() {
            let local_counter = self.local_vc.get(node_id);

            // Special case: operation from same node must be exactly next in sequence
            if node_id == &op.cluster_node_id {
                // We should have seen exactly op_counter - 1 operations from this node
                if local_counter != op_counter - 1 {
                    debug!(
                        op_id = %op.op_id,
                        node_id = %node_id,
                        expected = op_counter - 1,
                        actual = local_counter,
                        "Operation from same node is not next in sequence"
                    );
                    return false;
                }
            } else {
                // For other nodes, we need to have seen at least this many operations
                if local_counter < *op_counter {
                    debug!(
                        op_id = %op.op_id,
                        node_id = %node_id,
                        required = op_counter,
                        actual = local_counter,
                        "Missing dependency from node"
                    );
                    return false;
                }
            }
        }

        true
    }

    /// Buffer an operation until its dependencies are satisfied
    pub(super) fn buffer_operation(&mut self, op: Operation) {
        // Check buffer size limit
        if self.buffered_ops.len() >= self.max_buffer_size {
            self.metrics.buffer_full_events.increment();
            warn!(
                current_size = self.buffered_ops.len(),
                max_size = self.max_buffer_size,
                op_id = %op.op_id,
                "Causal delivery buffer is full - possible network partition or slow peer"
            );
        }

        // Calculate which cluster nodes this operation depends on
        let mut depends_on = HashSet::new();
        for (node_id, op_counter) in op.vector_clock.as_map() {
            let local_counter = self.local_vc.get(node_id);
            if local_counter < *op_counter {
                depends_on.insert(node_id.clone());
            }
        }

        debug!(
            op_id = %op.op_id,
            cluster_node = %op.cluster_node_id,
            depends_on = ?depends_on,
            "Buffering operation waiting for dependencies"
        );

        let buffered_at = Instant::now();
        self.buffered_ops
            .insert(op.op_id, (op, depends_on, buffered_at));
        self.stats.total_buffered += 1;
        self.stats.dependency_waits += 1;
        self.stats.current_buffered = self.buffered_ops.len();

        // Update atomic metrics
        self.metrics.total_buffered.increment();
        self.metrics.current_size.increment();

        let new_size = self.buffered_ops.len();
        if new_size > self.stats.max_buffered {
            self.stats.max_buffered = new_size;
        }
        if new_size > self.metrics.max_size_seen.get() {
            self.metrics.max_size_seen.set(new_size);
        }
    }

    /// Update local vector clock after delivering an operation
    pub(super) fn update_local_clock(&mut self, op: &Operation) {
        // Merge the operation's vector clock into our local clock
        self.local_vc.merge(&op.vector_clock);
    }

    /// Check buffered operations and deliver any that are now ready
    ///
    /// Returns operations in causal order
    pub(super) fn check_buffered_ops(&mut self) -> Vec<Operation> {
        let mut deliverable = Vec::new();

        // Keep checking until no more operations can be delivered
        // (Delivering one operation might enable others)
        loop {
            let mut delivered_any = false;

            // Find operations whose dependencies are now satisfied
            let ready_ops: Vec<Uuid> = self
                .buffered_ops
                .iter()
                .filter(|(_, (op, _, _))| self.dependencies_satisfied(op))
                .map(|(op_id, _)| *op_id)
                .collect();

            if ready_ops.is_empty() {
                break;
            }

            // Deliver ready operations
            for op_id in ready_ops {
                if let Some((op, _, buffered_at)) = self.buffered_ops.remove(&op_id) {
                    debug!(
                        op_id = %op.op_id,
                        cluster_node = %op.cluster_node_id,
                        "Delivering buffered operation (dependencies satisfied)"
                    );

                    // Record delivery lag
                    let lag = buffered_at.elapsed();
                    self.metrics.delivery_lag.record(lag);

                    self.update_local_clock(&op);
                    deliverable.push(op);
                    delivered_any = true;

                    self.stats.total_delivered += 1;
                    self.stats.current_buffered = self.buffered_ops.len();

                    // Update atomic metrics
                    self.metrics.total_delivered.increment();
                    self.metrics.current_size.decrement();
                }
            }

            if !delivered_any {
                break;
            }
        }

        // Sort deliverable operations by causal order before returning
        deliverable.sort_by(|a, b| {
            if a.vector_clock.happens_before(&b.vector_clock) {
                std::cmp::Ordering::Less
            } else if a.vector_clock.happens_after(&b.vector_clock) {
                std::cmp::Ordering::Greater
            } else {
                // Concurrent - use timestamp tie-breaker
                a.timestamp_ms.cmp(&b.timestamp_ms)
            }
        });

        if !deliverable.is_empty() {
            info!(
                count = deliverable.len(),
                "Delivered buffered operations after dependency resolution"
            );
        }

        deliverable
    }

    /// Get comprehensive metrics about the causal delivery buffer
    ///
    /// This provides a snapshot of buffer state, performance, and health metrics.
    /// Metrics are collected using atomic counters with minimal overhead (<1%).
    pub fn get_metrics(&self) -> CausalBufferMetrics {
        let current_size = self.buffered_ops.len();
        let utilization = if self.max_buffer_size > 0 {
            (current_size as f64 / self.max_buffer_size as f64) * 100.0
        } else {
            0.0
        };

        // Calculate age of oldest buffered operation
        let oldest_age_ms = self
            .buffered_ops
            .values()
            .map(|(_, _, buffered_at)| buffered_at.elapsed().as_millis() as u64)
            .max()
            .unwrap_or(0);

        // Count operations with missing dependencies
        let missing_deps = self
            .buffered_ops
            .values()
            .filter(|(_, depends_on, _)| !depends_on.is_empty())
            .count();

        CausalBufferMetrics {
            current_size,
            max_size: self.max_buffer_size,
            utilization_percent: utilization,
            operations_delivered: self.metrics.total_delivered.get(),
            operations_buffered: self.metrics.total_buffered.get(),
            avg_delivery_lag_ms: self.metrics.delivery_lag.avg_ms(),
            oldest_op_age_ms: oldest_age_ms,
            missing_dependencies: missing_deps,
            buffer_full_events: self.metrics.buffer_full_events.get(),
            direct_deliveries: self.metrics.direct_deliveries.get(),
            p50_delivery_lag_ms: self.metrics.delivery_lag.percentile(50.0).as_millis() as u64,
            p99_delivery_lag_ms: self.metrics.delivery_lag.percentile(99.0).as_millis() as u64,
            timestamp: current_timestamp_ms(),
        }
    }
}
