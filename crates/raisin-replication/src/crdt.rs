use crate::operation::{OpType, Operation};
use crate::vector_clock::{ClockOrdering, VectorClock};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Result of merging operations using CRDT rules
#[derive(Debug, Clone)]
pub enum MergeResult {
    /// The winning operation after merge
    Winner(Operation),
    /// A conflict was detected (even if auto-resolved)
    Conflict {
        winner: Operation,
        losers: Vec<Operation>,
        conflict_type: ConflictType,
    },
}

/// Types of conflicts that can occur
///
/// The per-property, move, list and delete-wins conflicts belonged to the
/// pre-v2 granular node ops (gone, plan "Phase 11d"): a node write is now one
/// snapshot register, merged last-write-wins, and delete-wins is decided by
/// the applicator's tombstones, not here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Concurrent writes of the same register (a schema entity, a node
    /// snapshot, a branch, ...)
    ConcurrentSchemaUpdate,
}

/// CRDT merge rules implementation
pub struct CrdtMerge;

impl CrdtMerge {
    /// Merge multiple operations targeting the same entity using CRDT rules
    pub fn merge_operations(ops: Vec<Operation>) -> MergeResult {
        if ops.is_empty() {
            panic!("Cannot merge empty operation list");
        }

        if ops.len() == 1 {
            return MergeResult::Winner(ops.into_iter().next().unwrap());
        }

        // All operations should target the same entity
        let target = ops[0].target();
        debug_assert!(ops.iter().all(|op| op.target() == target));

        match &ops[0].op_type {
            OpType::AddRelation { .. } | OpType::RemoveRelation { .. } => {
                Self::merge_relation_operations(ops)
            }
            _ => Self::merge_last_write_wins(ops),
        }
    }

    /// Merge relation operations using Last-Write-Wins (LWW) CRDT
    ///
    /// CRDT Rule: The most recent operation (by vector clock) wins.
    /// Relations are identified by the composite key (source_id, target_id, relation_type).
    /// Only one relation of a given type can exist between two nodes.
    /// `ops` is non-empty (checked by [`Self::merge_operations`]).
    fn merge_relation_operations(ops: Vec<Operation>) -> MergeResult {
        // Simply return the operation with the latest vector clock (LWW)
        // All operations should have the same composite key (source, target, type)
        let winner = ops
            .into_iter()
            .max_by(Self::compare_operations_lww)
            .expect("ops is non-empty");

        MergeResult::Winner(winner)
    }

    /// Generic Last-Write-Wins merge for schema and other operations
    fn merge_last_write_wins(ops: Vec<Operation>) -> MergeResult {
        let winner = Self::select_lww_winner(&ops);
        let losers: Vec<_> = ops
            .into_iter()
            .filter(|op| op.op_id != winner.op_id)
            .collect();

        let has_concurrent = losers
            .iter()
            .any(|loser| winner.vector_clock.concurrent_with(&loser.vector_clock));

        if has_concurrent {
            MergeResult::Conflict {
                winner,
                losers,
                conflict_type: ConflictType::ConcurrentSchemaUpdate,
            }
        } else {
            MergeResult::Winner(winner)
        }
    }

    /// Select the Last-Write-Wins winner from a set of operations
    ///
    /// Three-level tie-breaking:
    /// 1. Vector clock (causal ordering)
    /// 2. Timestamp (wall clock)
    /// 3. Node ID (deterministic)
    fn select_lww_winner(ops: &[Operation]) -> Operation {
        ops.iter()
            .max_by(|a, b| Self::compare_operations_lww(a, b))
            .cloned()
            .unwrap()
    }

    /// Compare two operations for Last-Write-Wins ordering
    pub fn compare_operations_lww(a: &Operation, b: &Operation) -> Ordering {
        // 1. Check vector clock causality
        match a.vector_clock.compare(&b.vector_clock) {
            ClockOrdering::After => return Ordering::Greater,
            ClockOrdering::Before => return Ordering::Less,
            ClockOrdering::Equal | ClockOrdering::Concurrent => {
                // Continue to timestamp
            }
        }

        // 2. Compare timestamps
        match a.timestamp_ms.cmp(&b.timestamp_ms) {
            Ordering::Greater => return Ordering::Greater,
            Ordering::Less => return Ordering::Less,
            Ordering::Equal => {
                // Continue to node ID
            }
        }

        // 3. Final deterministic tie-breaker: cluster node ID
        a.cluster_node_id.cmp(&b.cluster_node_id)
    }
}

impl From<ClockOrdering> for Ordering {
    fn from(clock_ord: ClockOrdering) -> Self {
        match clock_ord {
            ClockOrdering::Before => Ordering::Less,
            ClockOrdering::After => Ordering::Greater,
            ClockOrdering::Equal => Ordering::Equal,
            ClockOrdering::Concurrent => Ordering::Equal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::RelationRef;
    use std::collections::HashSet;
    use uuid::Uuid;

    fn make_register_op(
        node_id: &str,
        op_seq: u64,
        vc: VectorClock,
        timestamp_ms: u64,
        property_value: &str,
    ) -> Operation {
        Operation {
            op_id: Uuid::new_v4(),
            op_seq,
            cluster_node_id: node_id.to_string(),
            timestamp_ms,
            vector_clock: vc,
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            // One register written twice: a tag moved to two revisions.
            op_type: OpType::CreateTag {
                tag_name: "release".to_string(),
                revision: property_value.to_string(),
            },
            revision: None,
            actor: "user".to_string(),
            message: None,
            is_system: false,
            agent: None,
            acknowledged_by: HashSet::new(),
        }
    }

    #[test]
    fn test_lww_causal_ordering() {
        // Operation 1 happens before Operation 2 causally
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 2);

        let op1 = make_register_op("node1", 1, vc1, 1000, "Value 1");
        let op2 = make_register_op("node1", 2, vc2, 1000, "Value 2");

        // op2 should win (happened after)
        let result = CrdtMerge::merge_operations(vec![op1.clone(), op2.clone()]);
        match result {
            MergeResult::Winner(winner) => {
                assert_eq!(winner.op_id, op2.op_id);
            }
            _ => panic!("Expected Winner"),
        }
    }

    #[test]
    fn test_lww_concurrent_timestamp_wins() {
        // Two concurrent operations, different timestamps
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);

        let op1 = make_register_op("node1", 1, vc1, 1000, "Value 1");
        let op2 = make_register_op("node2", 1, vc2, 2000, "Value 2");

        // op2 should win (later timestamp)
        let result = CrdtMerge::merge_operations(vec![op1.clone(), op2.clone()]);
        match result {
            MergeResult::Conflict { winner, .. } => {
                assert_eq!(winner.op_id, op2.op_id);
            }
            _ => panic!("Expected Conflict"),
        }
    }

    #[test]
    fn test_lww_concurrent_node_id_tiebreaker() {
        // Two concurrent operations, same timestamp
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);

        let op1 = make_register_op("node1", 1, vc1, 1000, "Value 1");
        let op2 = make_register_op("node2", 1, vc2, 1000, "Value 2");

        // node2 > node1 lexicographically, so op2 wins
        let result = CrdtMerge::merge_operations(vec![op1.clone(), op2.clone()]);
        match result {
            MergeResult::Conflict { winner, .. } => {
                assert_eq!(winner.op_id, op2.op_id);
            }
            _ => panic!("Expected Conflict"),
        }
    }

    #[test]
    fn test_lww_relation() {
        let mut vc_add = VectorClock::new();
        vc_add.set("node1", 1);

        let mut vc_remove = VectorClock::new();
        vc_remove.set("node2", 1); // Concurrent

        let add_op = Operation {
            op_id: Uuid::new_v4(),
            op_seq: 1,
            cluster_node_id: "node1".to_string(),
            timestamp_ms: 1000,
            vector_clock: vc_add,
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::AddRelation {
                source_id: "source".to_string(),
                source_workspace: "workspace".to_string(),
                relation_type: "refs".to_string(),
                target_id: "target".to_string(),
                target_workspace: "workspace".to_string(),
                relation: RelationRef::new(
                    "target".to_string(),
                    "workspace".to_string(),
                    "".to_string(),
                    "refs".to_string(),
                    None,
                ),
            },
            revision: None,
            actor: "user".to_string(),
            message: None,
            is_system: false,
            agent: None,
            acknowledged_by: HashSet::new(),
        };

        let remove_op = Operation {
            op_id: Uuid::new_v4(),
            op_seq: 1,
            cluster_node_id: "node2".to_string(),
            timestamp_ms: 1000,
            vector_clock: vc_remove,
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::RemoveRelation {
                source_id: "source".to_string(),
                source_workspace: "workspace".to_string(),
                relation_type: "refs".to_string(),
                target_id: "target".to_string(),
                target_workspace: "workspace".to_string(),
            },
            revision: None,
            actor: "user".to_string(),
            message: None,
            is_system: false,
            agent: None,
            acknowledged_by: HashSet::new(),
        };

        // LWW: With concurrent operations and same timestamp, result is deterministic
        // based on vector clock comparison
        let result = CrdtMerge::merge_operations(vec![add_op.clone(), remove_op]);
        match result {
            MergeResult::Winner(_winner) => {
                // One of the operations should win based on LWW comparison
                // The test just verifies no panic occurs
            }
            _ => panic!("Expected Winner for LWW"),
        }
    }
}
