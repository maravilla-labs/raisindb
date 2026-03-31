//! Metrics for operation decomposition
//!
//! This module provides a stateful wrapper around the stateless decompose_operation
//! function to track decomposition metrics.

use crate::metrics::{AtomicCounter, DecompositionMetrics, DurationHistogram};
use crate::operation::{OpType, Operation};
use std::time::Instant;

/// Operation decomposer with metrics tracking
///
/// This wraps the stateless `decompose_operation` function with metrics collection.
#[derive(Debug, Clone)]
pub struct OperationDecomposer {
    metrics: DecomposerMetrics,
}

/// Metrics for operation decomposer
#[derive(Debug, Clone)]
struct DecomposerMetrics {
    operations_in: AtomicCounter,
    operations_out: AtomicCounter,
    apply_revision_count: AtomicCounter,
    upsert_snapshot_count: AtomicCounter,
    delete_snapshot_count: AtomicCounter,
    passthrough_count: AtomicCounter,
    decomposition_duration: DurationHistogram,
}

impl OperationDecomposer {
    /// Create a new operation decomposer with metrics tracking
    pub fn new() -> Self {
        Self {
            metrics: DecomposerMetrics {
                operations_in: AtomicCounter::new(),
                operations_out: AtomicCounter::new(),
                apply_revision_count: AtomicCounter::new(),
                upsert_snapshot_count: AtomicCounter::new(),
                delete_snapshot_count: AtomicCounter::new(),
                passthrough_count: AtomicCounter::new(),
                decomposition_duration: DurationHistogram::new(1000),
            },
        }
    }

    /// Decompose an operation into atomic, commutative operations
    ///
    /// This delegates to the stateless `decompose_operation` function
    /// while tracking metrics.
    ///
    /// # Arguments
    /// * `op` - The operation to potentially decompose
    ///
    /// # Returns
    /// Vector of atomic operations (may be just the original operation)
    pub fn decompose(&self, op: Operation) -> Vec<Operation> {
        let start = Instant::now();
        self.metrics.operations_in.increment();

        // Decompose the operation
        let result = crate::operation_decomposer::decompose_operation(op.clone());
        let output_count = result.len();

        // Track metrics based on operation type
        match &op.op_type {
            OpType::ApplyRevision { node_changes, .. } => {
                self.metrics.apply_revision_count.increment();

                // Count upserts and deletes
                for change in node_changes {
                    match &change.kind {
                        crate::operation::ReplicatedNodeChangeKind::Upsert => {
                            self.metrics.upsert_snapshot_count.increment();
                        }
                        crate::operation::ReplicatedNodeChangeKind::Delete => {
                            self.metrics.delete_snapshot_count.increment();
                        }
                    }
                }
            }
            _ => {
                self.metrics.passthrough_count.increment();
            }
        }

        self.metrics.operations_out.add(output_count as u64);
        self.metrics.decomposition_duration.record(start.elapsed());

        result
    }

    /// Decompose a batch of operations
    ///
    /// More efficient than calling decompose() multiple times as it tracks
    /// metrics in a batch.
    ///
    /// # Arguments
    /// * `ops` - Operations to decompose
    ///
    /// # Returns
    /// Vector of all decomposed atomic operations
    pub fn decompose_batch(&self, ops: Vec<Operation>) -> Vec<Operation> {
        let mut result = Vec::with_capacity(ops.len() * 2); // Estimate 2x expansion
        for op in ops {
            result.extend(self.decompose(op));
        }
        result
    }

    /// Get metrics for this decomposer
    pub fn get_metrics(&self) -> DecompositionMetrics {
        let ops_in = self.metrics.operations_in.get();
        let ops_out = self.metrics.operations_out.get();

        let expansion_ratio = if ops_in > 0 {
            ops_out as f64 / ops_in as f64
        } else {
            0.0
        };

        DecompositionMetrics {
            operations_in: ops_in,
            operations_out: ops_out,
            expansion_ratio,
            avg_duration_ms: self.metrics.decomposition_duration.avg_ms(),
            apply_revision_count: self.metrics.apply_revision_count.get(),
            upsert_snapshot_count: self.metrics.upsert_snapshot_count.get(),
            delete_snapshot_count: self.metrics.delete_snapshot_count.get(),
            passthrough_count: self.metrics.passthrough_count.get(),
            p99_duration_ms: self
                .metrics
                .decomposition_duration
                .percentile(99.0)
                .as_millis() as u64,
            timestamp: crate::metrics::current_timestamp_ms(),
        }
    }

    /// Reset metrics to zero
    ///
    /// Useful for testing or periodic metric windows
    pub fn reset_metrics(&self) {
        self.metrics.operations_in.set(0);
        self.metrics.operations_out.set(0);
        self.metrics.apply_revision_count.set(0);
        self.metrics.upsert_snapshot_count.set(0);
        self.metrics.delete_snapshot_count.set(0);
        self.metrics.passthrough_count.set(0);
    }
}

impl Default for OperationDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpType, VectorClock};
    use raisin_models::nodes::Node;
    use uuid::Uuid;

    fn make_test_op(op_type: OpType) -> Operation {
        Operation {
            op_id: Uuid::new_v4(),
            op_seq: 1,
            cluster_node_id: "node1".to_string(),
            timestamp_ms: 1000,
            vector_clock: VectorClock::new(),
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            op_type,
            revision: None,
            actor: "user".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: Default::default(),
        }
    }

    #[test]
    fn test_decomposer_tracks_passthrough() {
        let decomposer = OperationDecomposer::new();

        let op = make_test_op(OpType::SetProperty {
            node_id: "test".to_string(),
            property_name: "title".to_string(),
            value: raisin_models::nodes::properties::PropertyValue::String("Test".to_string()),
        });

        let result = decomposer.decompose(op);
        assert_eq!(result.len(), 1);

        let metrics = decomposer.get_metrics();
        assert_eq!(metrics.operations_in, 1);
        assert_eq!(metrics.operations_out, 1);
        assert_eq!(metrics.passthrough_count, 1);
        assert_eq!(metrics.expansion_ratio, 1.0);
    }

    #[test]
    fn test_decomposer_tracks_expansion_ratio() {
        let decomposer = OperationDecomposer::new();

        // Simulate ApplyRevision with 3 node changes
        use crate::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};
        use raisin_hlc::HLC;

        let node_changes = vec![
            ReplicatedNodeChange {
                node: Node {
                    id: "node1".to_string(),
                    name: "test".to_string(),
                    node_type: "Document".to_string(),
                    ..Default::default()
                },
                parent_id: None,
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: String::new(),
            },
            ReplicatedNodeChange {
                node: Node {
                    id: "node2".to_string(),
                    name: "test2".to_string(),
                    node_type: "Document".to_string(),
                    ..Default::default()
                },
                parent_id: None,
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: String::new(),
            },
            ReplicatedNodeChange {
                node: Node {
                    id: "node3".to_string(),
                    name: "test3".to_string(),
                    node_type: "Document".to_string(),
                    ..Default::default()
                },
                parent_id: None,
                kind: ReplicatedNodeChangeKind::Delete,
                cf_order_key: String::new(),
            },
        ];

        let op = make_test_op(OpType::ApplyRevision {
            branch_head: HLC::new(100, 0),
            node_changes,
        });

        let result = decomposer.decompose(op);
        assert_eq!(result.len(), 3);

        let metrics = decomposer.get_metrics();
        assert_eq!(metrics.operations_in, 1);
        assert_eq!(metrics.operations_out, 3);
        assert_eq!(metrics.apply_revision_count, 1);
        assert_eq!(metrics.upsert_snapshot_count, 2);
        assert_eq!(metrics.delete_snapshot_count, 1);
        assert_eq!(metrics.expansion_ratio, 3.0);
    }
}
