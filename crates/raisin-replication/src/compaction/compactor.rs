//! Operation log compactor implementation
//!
//! Intelligently reduces the size of the operation log while preserving
//! CRDT semantics and eventual consistency guarantees.

use crate::operation::{OpType, Operation};
use hashbrown::HashMap;

use super::config::{CompactionConfig, CompactionResult, NodeCompactionStats};

/// Operation log compactor
///
/// This compactor intelligently reduces the size of the operation log
/// while preserving CRDT semantics and eventual consistency guarantees.
pub struct OperationLogCompactor {
    config: CompactionConfig,
}

impl OperationLogCompactor {
    /// Create a new compactor with the given configuration
    pub fn new(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Create a compactor with default configuration
    pub fn default_config() -> Self {
        Self::new(CompactionConfig::default())
    }

    /// Compact operations for a specific cluster node
    ///
    /// This method takes a list of operations from a single cluster node
    /// and reduces them by merging redundant operations.
    ///
    /// # Safety
    ///
    /// This method assumes all operations are from the **same cluster node**.
    /// Mixing operations from different cluster nodes will violate causality!
    pub fn compact_node_operations(
        &self,
        operations: Vec<Operation>,
        current_time_ms: u64,
    ) -> (Vec<Operation>, CompactionResult) {
        let original_count = operations.len();

        if operations.is_empty() {
            return (
                operations,
                CompactionResult {
                    original_count: 0,
                    compacted_count: 0,
                    merged_count: 0,
                    bytes_saved: 0,
                    per_node_stats: HashMap::new(),
                },
            );
        }

        // Verify all operations are from the same cluster node (safety check)
        let cluster_node_id = operations[0].cluster_node_id.clone();
        debug_assert!(
            operations
                .iter()
                .all(|op| op.cluster_node_id == cluster_node_id),
            "All operations must be from the same cluster node"
        );

        // Separate operations into compactable and non-compactable
        let (old_ops, recent_ops): (Vec<_>, Vec<_>) = operations
            .into_iter()
            .partition(|op| self.is_old_enough(op, current_time_ms));

        // Group compactable operations by compaction key
        let mut grouped: HashMap<CompactionKey, Vec<Operation>> = HashMap::new();

        for op in old_ops {
            if let Some(key) = self.get_compaction_key(&op) {
                grouped.entry(key).or_default().push(op);
            } else {
                // Operations that can't be compacted are added as singletons
                let singleton_key = CompactionKey::singleton(&op);
                grouped.insert(singleton_key, vec![op]);
            }
        }

        // Compact each group
        let mut compacted = Vec::with_capacity(grouped.len() + recent_ops.len());
        let mut property_sequences_merged = 0;

        for (key, mut ops) in grouped {
            if ops.len() > 1 && key.is_mergeable() && self.config.merge_property_updates {
                // Sort by op_seq to ensure chronological order
                ops.sort_by_key(|op| op.op_seq);

                // Keep only the last operation in the sequence
                if let Some(last_op) = ops.last() {
                    compacted.push(last_op.clone());
                    property_sequences_merged += 1;
                }
            } else {
                compacted.extend(ops);
            }
        }

        // Add all recent operations (not eligible for compaction)
        compacted.extend(recent_ops);

        // Sort final result by op_seq to maintain chronological order
        compacted.sort_by_key(|op| op.op_seq);

        let compacted_count = compacted.len();
        let merged_count = original_count.saturating_sub(compacted_count);
        let bytes_saved = Self::estimate_bytes_saved(original_count, compacted_count);

        let mut per_node_stats = HashMap::new();
        per_node_stats.insert(
            cluster_node_id,
            NodeCompactionStats {
                original_count,
                compacted_count,
                property_sequences_merged,
            },
        );

        let result = CompactionResult {
            original_count,
            compacted_count,
            merged_count,
            bytes_saved,
            per_node_stats,
        };

        (compacted, result)
    }

    /// Check if an operation is old enough to be considered for compaction
    fn is_old_enough(&self, op: &Operation, current_time_ms: u64) -> bool {
        let age_ms = current_time_ms.saturating_sub(op.timestamp_ms);
        let age_secs = age_ms / 1000;
        age_secs >= self.config.min_age_secs
    }

    /// Get the compaction key for an operation
    ///
    /// Operations with the same compaction key can potentially be merged.
    /// Returns None for operations that should never be compacted.
    fn get_compaction_key(&self, op: &Operation) -> Option<CompactionKey> {
        match &op.op_type {
            OpType::SetProperty {
                node_id,
                property_name,
                ..
            } => Some(CompactionKey::Property {
                cluster_node_id: op.cluster_node_id.clone(),
                storage_node_id: node_id.clone(),
                property_name: property_name.clone(),
            }),
            // These operation types should NOT be compacted
            _ => None,
        }
    }

    /// Estimate bytes saved by compaction
    fn estimate_bytes_saved(original_count: usize, compacted_count: usize) -> usize {
        const AVG_OP_SIZE: usize = 512;
        let removed_count = original_count.saturating_sub(compacted_count);
        removed_count * AVG_OP_SIZE
    }
}

/// Key used to group operations for compaction
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum CompactionKey {
    /// SetProperty operations on the same property of the same storage node
    Property {
        cluster_node_id: String,
        storage_node_id: String,
        property_name: String,
    },

    /// Singleton key for non-mergeable operations
    Singleton {
        cluster_node_id: String,
        op_seq: u64,
    },
}

impl CompactionKey {
    /// Check if operations with this key can be merged
    fn is_mergeable(&self) -> bool {
        matches!(self, CompactionKey::Property { .. })
    }

    /// Create a singleton key for a non-mergeable operation
    fn singleton(op: &Operation) -> Self {
        CompactionKey::Singleton {
            cluster_node_id: op.cluster_node_id.clone(),
            op_seq: op.op_seq,
        }
    }
}
