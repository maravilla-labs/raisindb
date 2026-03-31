use crate::operation::{OpType, Operation};
use crate::vector_clock::{ClockOrdering, VectorClock};
use hashbrown::HashMap;
use raisin_models::nodes::properties::PropertyValue;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use uuid::Uuid;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Concurrent property updates on the same property
    ConcurrentPropertyUpdate,
    /// Concurrent moves of the same node to different parents
    ConcurrentMove,
    /// Concurrent updates to same schema
    ConcurrentSchemaUpdate,
    /// Delete concurrent with update
    DeleteWinsOverUpdate,
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

        // Group operations by type and apply appropriate CRDT rule
        match &ops[0].op_type {
            OpType::SetProperty { .. } | OpType::DeleteProperty { .. } => {
                Self::merge_property_operations(ops)
            }
            OpType::AddRelation { .. } | OpType::RemoveRelation { .. } => {
                Self::merge_relation_operations(ops)
            }
            OpType::MoveNode { .. } => Self::merge_move_operations(ops),
            OpType::ListInsertAfter { .. } | OpType::ListDelete { .. } => {
                Self::merge_list_operations(ops)
            }
            OpType::DeleteNode { .. } => Self::merge_delete_operations(ops),
            _ => Self::merge_last_write_wins(ops),
        }
    }

    /// Merge property operations using Last-Write-Wins with vector clock ordering
    ///
    /// CRDT Rule: LWW with three-level tie-breaking:
    /// 1. Vector clock (causal ordering)
    /// 2. Timestamp (wall clock)
    /// 3. Node ID (deterministic)
    fn merge_property_operations(ops: Vec<Operation>) -> MergeResult {
        let winner = Self::select_lww_winner(&ops);
        let losers: Vec<_> = ops
            .into_iter()
            .filter(|op| op.op_id != winner.op_id)
            .collect();

        if !losers.is_empty() {
            // Check if any losers were concurrent with winner
            let has_concurrent = losers
                .iter()
                .any(|loser| winner.vector_clock.concurrent_with(&loser.vector_clock));

            if has_concurrent {
                return MergeResult::Conflict {
                    winner,
                    losers,
                    conflict_type: ConflictType::ConcurrentPropertyUpdate,
                };
            }
        }

        MergeResult::Winner(winner)
    }

    /// Merge relation operations using Last-Write-Wins (LWW) CRDT
    ///
    /// CRDT Rule: The most recent operation (by vector clock) wins.
    /// Relations are identified by the composite key (source_id, target_id, relation_type).
    /// Only one relation of a given type can exist between two nodes.
    fn merge_relation_operations(ops: Vec<Operation>) -> MergeResult {
        if ops.is_empty() {
            // No operations to merge - create a no-op result
            return MergeResult::Winner(Operation::new(
                0,
                "system".to_string(),
                VectorClock::new(),
                String::new(),
                String::new(),
                String::new(),
                OpType::DeleteNode {
                    node_id: String::new(),
                },
                "system".to_string(),
            ));
        }

        // Simply return the operation with the latest vector clock (LWW)
        // All operations should have the same composite key (source, target, type)
        let winner = ops
            .into_iter()
            .max_by(Self::compare_operations_lww)
            .expect("ops is non-empty");

        MergeResult::Winner(winner)
    }

    /// Merge move operations using Last-Write-Wins
    ///
    /// CRDT Rule: LWW with conflict detection
    /// The node ends up in ONE location (winner's parent)
    fn merge_move_operations(ops: Vec<Operation>) -> MergeResult {
        let winner = Self::select_lww_winner(&ops);
        let losers: Vec<_> = ops
            .into_iter()
            .filter(|op| op.op_id != winner.op_id)
            .collect();

        // Check for concurrent moves to different parents
        let has_concurrent_different_parent = losers.iter().any(|loser| {
            if !winner.vector_clock.concurrent_with(&loser.vector_clock) {
                return false;
            }

            // Extract parent IDs
            if let (
                OpType::MoveNode {
                    new_parent_id: winner_parent,
                    ..
                },
                OpType::MoveNode {
                    new_parent_id: loser_parent,
                    ..
                },
            ) = (&winner.op_type, &loser.op_type)
            {
                winner_parent != loser_parent
            } else {
                false
            }
        });

        if has_concurrent_different_parent {
            MergeResult::Conflict {
                winner,
                losers,
                conflict_type: ConflictType::ConcurrentMove,
            }
        } else {
            MergeResult::Winner(winner)
        }
    }

    /// Merge ordered list operations using RGA (Replicated Growable Array) CRDT
    ///
    /// CRDT Rule: RGA with tombstones
    /// Elements are ordered by their insertion position and vector clock
    fn merge_list_operations(ops: Vec<Operation>) -> MergeResult {
        // Build RGA structure
        let mut elements: HashMap<Uuid, RGAElement> = HashMap::new();

        for op in ops {
            match &op.op_type {
                OpType::ListInsertAfter {
                    element_id,
                    after_id,
                    value,
                    ..
                } => {
                    elements.insert(
                        *element_id,
                        RGAElement {
                            id: *element_id,
                            value: value.clone(),
                            after_id: *after_id,
                            vector_clock: op.vector_clock.clone(),
                            tombstone: false,
                            insert_op: op,
                        },
                    );
                }
                OpType::ListDelete { element_id, .. } => {
                    if let Some(elem) = elements.get_mut(element_id) {
                        elem.tombstone = true;
                    }
                }
                _ => {}
            }
        }

        // Return the most recent insert as the "winner" for reporting purposes
        let winner = elements
            .values()
            .filter(|e| !e.tombstone)
            .max_by(|a, b| Self::compare_operations_lww(&a.insert_op, &b.insert_op))
            .map(|e| e.insert_op.clone())
            .unwrap_or_else(|| {
                // All deleted - return a delete operation
                elements
                    .values()
                    .max_by(|a, b| a.vector_clock.compare(&b.vector_clock).into())
                    .map(|e| e.insert_op.clone())
                    .unwrap()
            });

        MergeResult::Winner(winner)
    }

    /// Merge delete operations with other operations
    ///
    /// CRDT Rule: Delete-Wins
    /// Deletions always win over concurrent updates to prevent resurrection
    fn merge_delete_operations(ops: Vec<Operation>) -> MergeResult {
        // Find the most recent delete
        let delete_op = ops
            .iter()
            .filter(|op| op.is_delete())
            .max_by(|a, b| Self::compare_operations_lww(a, b))
            .cloned();

        if let Some(delete) = delete_op {
            let updates: Vec<_> = ops
                .into_iter()
                .filter(|op| {
                    !op.is_delete() && delete.vector_clock.concurrent_with(&op.vector_clock)
                })
                .collect();

            if !updates.is_empty() {
                return MergeResult::Conflict {
                    winner: delete,
                    losers: updates,
                    conflict_type: ConflictType::DeleteWinsOverUpdate,
                };
            }

            MergeResult::Winner(delete)
        } else {
            // No deletes, use LWW for updates
            Self::merge_last_write_wins(ops)
        }
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

/// RGA (Replicated Growable Array) element for ordered lists
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct RGAElement {
    id: Uuid,
    value: PropertyValue,
    after_id: Option<Uuid>,
    vector_clock: VectorClock,
    tombstone: bool,
    insert_op: Operation,
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

    fn make_set_property_op(
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
            op_type: OpType::SetProperty {
                node_id: "node123".to_string(),
                property_name: "title".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::String(
                    property_value.to_string(),
                ),
            },
            revision: None,
            actor: "user".to_string(),
            message: None,
            is_system: false,
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

        let op1 = make_set_property_op("node1", 1, vc1, 1000, "Value 1");
        let op2 = make_set_property_op("node1", 2, vc2, 1000, "Value 2");

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

        let op1 = make_set_property_op("node1", 1, vc1, 1000, "Value 1");
        let op2 = make_set_property_op("node2", 1, vc2, 2000, "Value 2");

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

        let op1 = make_set_property_op("node1", 1, vc1, 1000, "Value 1");
        let op2 = make_set_property_op("node2", 1, vc2, 1000, "Value 2");

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
