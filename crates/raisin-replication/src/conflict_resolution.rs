//! Conflict resolution for masterless P2P cluster
//!
//! This module provides deterministic conflict resolution when multiple nodes
//! have divergent operation logs due to concurrent writes.

use crate::{Operation, VectorClock};
use std::cmp::Ordering;
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Conflict resolver using vector clock consensus
pub struct ConflictResolver {
    /// Local node ID
    local_node_id: String,
}

impl ConflictResolver {
    /// Create a new conflict resolver
    pub fn new(local_node_id: String) -> Self {
        Self { local_node_id }
    }

    /// Resolve divergent operation logs from multiple peers using vector clock consensus
    ///
    /// This method merges operations from all peers and establishes a deterministic
    /// total order based on:
    /// 1. Vector clock (happens-before relationship)
    /// 2. Timestamp (for concurrent operations)
    /// 3. Node ID (for tie-breaking)
    ///
    /// # Arguments
    /// * `local_ops` - Operations from the local node
    /// * `peer_logs` - Operations from each peer, keyed by peer ID
    ///
    /// # Returns
    /// A deduplicated, deterministically ordered list of operations
    pub fn resolve_divergent_logs(
        &self,
        local_ops: Vec<Operation>,
        peer_logs: HashMap<String, Vec<Operation>>,
    ) -> Result<Vec<Operation>, ConflictError> {
        info!(
            local_ops = local_ops.len(),
            peers = peer_logs.len(),
            "Resolving divergent operation logs"
        );

        // Step 1: Merge all operations from all sources
        let mut all_ops = local_ops.clone();
        let mut peer_count = HashMap::new();

        for (peer_id, ops) in peer_logs {
            peer_count.insert(peer_id.clone(), ops.len());
            all_ops.extend(ops);
        }

        debug!(
            total_ops = all_ops.len(),
            "Merged operations from all sources"
        );

        // Step 2: Deduplicate by operation ID (same operation may be in multiple logs)
        let mut unique_ops: HashMap<Uuid, Operation> = HashMap::new();
        let mut duplicates = 0;

        for op in all_ops {
            if unique_ops.insert(op.op_id, op).is_some() {
                duplicates += 1;
            }
        }

        debug!(
            unique_ops = unique_ops.len(),
            duplicates = duplicates,
            "Deduplicated operations"
        );

        // Step 3: Convert to vec and sort by deterministic order
        let mut ops: Vec<Operation> = unique_ops.into_values().collect();
        ops.sort_by(|a, b| self.compare_operations(a, b));

        info!(final_ops = ops.len(), "Conflict resolution complete");

        Ok(ops)
    }

    /// Compare two operations to establish deterministic total order
    ///
    /// Order is determined by:
    /// 1. Vector clock (causal order)
    /// 2. Timestamp (for concurrent operations)
    /// 3. Node ID (lexicographic, for tie-breaking)
    ///
    /// This ensures all nodes converge to the same operation order.
    fn compare_operations(&self, a: &Operation, b: &Operation) -> Ordering {
        use crate::ClockOrdering;

        // First, try vector clock comparison
        match a.vector_clock.compare(&b.vector_clock) {
            ClockOrdering::Before => return Ordering::Less,
            ClockOrdering::After => return Ordering::Greater,
            ClockOrdering::Equal => {
                // Same vector clock - should be the same operation
                // But use timestamp as secondary order just in case
            }
            ClockOrdering::Concurrent => {
                // Concurrent operations - need tie-breaker
            }
        }

        // Second, compare timestamps
        match a.timestamp_ms.cmp(&b.timestamp_ms) {
            Ordering::Equal => {
                // Same timestamp - use node ID for determinism
                a.cluster_node_id.cmp(&b.cluster_node_id)
            }
            other => other,
        }
    }

    /// Detect conflicts between operations
    ///
    /// Returns operations that target the same entity but have concurrent vector clocks
    pub fn detect_conflicts(&self, ops: &[Operation]) -> Vec<ConflictGroup> {
        use crate::ClockOrdering;

        let mut conflicts = Vec::new();
        let mut by_target: HashMap<String, Vec<&Operation>> = HashMap::new();

        // Group operations by target
        for op in ops {
            let target_key = format!("{}/{}/{}", op.tenant_id, op.repo_id, op.branch);
            by_target.entry(target_key).or_default().push(op);
        }

        // Find concurrent operations on same target
        for (target, target_ops) in by_target {
            if target_ops.len() < 2 {
                continue;
            }

            let mut concurrent_groups = Vec::new();

            for i in 0..target_ops.len() {
                for j in (i + 1)..target_ops.len() {
                    let op_a = target_ops[i];
                    let op_b = target_ops[j];

                    if matches!(
                        op_a.vector_clock.compare(&op_b.vector_clock),
                        ClockOrdering::Concurrent
                    ) {
                        concurrent_groups.push(vec![op_a.clone(), op_b.clone()]);
                    }
                }
            }

            if !concurrent_groups.is_empty() {
                conflicts.push(ConflictGroup {
                    target,
                    concurrent_operations: concurrent_groups,
                });
            }
        }

        if !conflicts.is_empty() {
            warn!(
                num_conflicts = conflicts.len(),
                "Detected concurrent operations on same targets"
            );
        }

        conflicts
    }

    /// Calculate consensus vector clock from multiple peer clocks
    ///
    /// Returns the "maximum" vector clock that encompasses all peer clocks
    pub fn calculate_consensus_vector_clock(&self, peer_clocks: &[VectorClock]) -> VectorClock {
        let mut consensus = VectorClock::new();

        for peer_clock in peer_clocks {
            consensus.merge(peer_clock);
        }

        info!("Calculated consensus vector clock");
        consensus
    }
}

/// Group of conflicting operations
#[derive(Debug, Clone)]
pub struct ConflictGroup {
    /// Target identifier (tenant/repo/branch)
    pub target: String,

    /// Groups of concurrent operations
    pub concurrent_operations: Vec<Vec<Operation>>,
}

/// Conflict resolution errors
#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    #[error("Invalid operation log: {0}")]
    InvalidLog(String),

    #[error("Conflict resolution failed: {0}")]
    ResolutionFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpType;
    use raisin_models::nodes::types::node_type::NodeType;

    fn make_test_operation(cluster_node_id: &str, op_seq: u64, timestamp_ms: u64) -> Operation {
        let mut vc = VectorClock::new();
        vc.set(cluster_node_id, op_seq);

        Operation {
            op_id: Uuid::new_v4(),
            op_seq,
            cluster_node_id: cluster_node_id.to_string(),
            timestamp_ms,
            vector_clock: vc,
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::UpdateNodeType {
                node_type_id: "test_type".to_string(),
                node_type: NodeType {
                    id: None,
                    strict: None,
                    name: "TestType".to_string(),
                    extends: None,
                    mixins: vec![],
                    overrides: None,
                    description: None,
                    icon: None,
                    version: None,
                    properties: None,
                    allowed_children: vec![],
                    required_nodes: vec![],
                    initial_structure: None,
                    versionable: None,
                    publishable: None,
                    auditable: None,
                    indexable: None,
                    index_types: None,
                    created_at: None,
                    updated_at: None,
                    published_at: None,
                    published_by: None,
                    previous_version: None,
                    compound_indexes: None,
                    is_mixin: None,
                },
            },
            revision: None,
            actor: "test".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: std::collections::HashSet::new(),
        }
    }

    #[test]
    fn test_resolve_divergent_logs() {
        let resolver = ConflictResolver::new("node1".to_string());

        let op1 = make_test_operation("node1", 1, 1000);
        let op2 = make_test_operation("node2", 1, 2000);
        let op3 = make_test_operation("node3", 1, 3000);

        let local_ops = vec![op1.clone()];
        let mut peer_logs = HashMap::new();
        peer_logs.insert("node2".to_string(), vec![op2.clone()]);
        peer_logs.insert("node3".to_string(), vec![op3.clone(), op1.clone()]); // op1 duplicated

        let result = resolver
            .resolve_divergent_logs(local_ops, peer_logs)
            .unwrap();

        assert_eq!(result.len(), 3); // Deduplicated
    }

    #[test]
    fn test_compare_operations_by_timestamp() {
        let resolver = ConflictResolver::new("node1".to_string());

        let op1 = make_test_operation("node1", 1, 1000);
        let op2 = make_test_operation("node2", 1, 2000);

        // op1 has earlier timestamp
        assert_eq!(resolver.compare_operations(&op1, &op2), Ordering::Less);
    }

    #[test]
    fn test_compare_operations_by_node_id() {
        let resolver = ConflictResolver::new("node1".to_string());

        let op1 = make_test_operation("node_a", 1, 1000);
        let op2 = make_test_operation("node_b", 1, 1000);

        // Same timestamp, compare by node ID
        assert_eq!(resolver.compare_operations(&op1, &op2), Ordering::Less);
    }

    #[test]
    fn test_calculate_consensus_vector_clock() {
        let resolver = ConflictResolver::new("node1".to_string());

        let mut vc1 = VectorClock::new();
        vc1.set("node1", 5);
        vc1.set("node2", 3);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 3);
        vc2.set("node2", 7);

        let mut vc3 = VectorClock::new();
        vc3.set("node1", 4);
        vc3.set("node3", 2);

        let consensus = resolver.calculate_consensus_vector_clock(&[vc1, vc2, vc3]);

        // Should have max of each node
        assert_eq!(consensus.get("node1"), 5);
        assert_eq!(consensus.get("node2"), 7);
        assert_eq!(consensus.get("node3"), 2);
    }

    #[test]
    fn test_detect_conflicts() {
        let resolver = ConflictResolver::new("node1".to_string());

        // Create two concurrent operations (different vector clocks)
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);

        let mut op1 = make_test_operation("node1", 1, 1000);
        op1.vector_clock = vc1;

        let mut op2 = make_test_operation("node2", 1, 2000);
        op2.vector_clock = vc2;

        let ops = vec![op1, op2];
        let conflicts = resolver.detect_conflicts(&ops);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].concurrent_operations.len(), 1);
    }
}
