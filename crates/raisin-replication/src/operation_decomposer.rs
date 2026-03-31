//! Operation Decomposer for CRDT Commutativity
//!
//! This module decomposes complex batched operations (like ApplyRevision) into
//! atomic operations that are guaranteed to be commutative. This is critical for
//! operation-based CRDT convergence.
//!
//! ## Why ApplyRevision Needs Decomposition
//!
//! `ApplyRevision` contains a batched list of node changes (upserts and deletes).
//! These changes are:
//! 1. **Ordered** - within a revision, changes must be applied in sequence
//! 2. **Interdependent** - later changes may reference earlier ones
//! 3. **NOT Commutative** - applying two ApplyRevision operations in different
//!    orders can produce different results
//!
//! To achieve CRDT convergence, we must decompose these into atomic, commutative
//! operations before replication.
//!
//! ## Decomposition Strategy
//!
//! Instead of replicating the full `ApplyRevision` operation, we replicate the
//! individual node-level operations that compose it:
//!
//! ```ignore
//! ApplyRevision {
//!     branch_head: HLC(100, node1),
//!     node_changes: [
//!         Upsert(node_a),
//!         Upsert(node_b),
//!         Delete(node_c)
//!     ]
//! }
//!
//! Decomposes to:
//! [
//!     UpsertNodeSnapshot { node: node_a, revision: HLC(100, node1), ... },
//!     UpsertNodeSnapshot { node: node_b, revision: HLC(100, node1), ... },
//!     DeleteNodeSnapshot { node_id: node_c, revision: HLC(100, node1), ... },
//! ]
//! ```
//!
//! Each decomposed operation:
//! - Targets a single node (enabling per-node CRDT merging)
//! - Includes the revision HLC (enabling timestamp-based LWW)
//! - Is commutative with other decomposed operations

use crate::operation::{OpType, Operation, ReplicatedNodeChange, ReplicatedNodeChangeKind};
use raisin_hlc::HLC;
use std::collections::HashSet;
use uuid::Uuid;

/// Decompose an operation into atomic, commutative operations
///
/// For most operations, this returns the original operation unchanged.
/// For `ApplyRevision`, this decomposes it into individual node operations.
///
/// # Arguments
/// * `op` - The operation to potentially decompose
///
/// # Returns
/// Vector of atomic operations (may be just the original operation)
pub fn decompose_operation(op: Operation) -> Vec<Operation> {
    match &op.op_type {
        OpType::ApplyRevision {
            branch_head,
            node_changes,
        } => decompose_apply_revision(&op, *branch_head, node_changes),
        _ => {
            // All other operations are already atomic
            vec![op]
        }
    }
}

/// Decompose ApplyRevision into individual node operations
///
/// Each node change becomes a separate operation with the same vector clock
/// and revision timestamp, but targeting a single node.
fn decompose_apply_revision(
    original_op: &Operation,
    branch_head: HLC,
    node_changes: &[ReplicatedNodeChange],
) -> Vec<Operation> {
    let mut decomposed = Vec::with_capacity(node_changes.len());

    for (index, node_change) in node_changes.iter().enumerate() {
        // Create a new operation ID for each decomposed operation
        // We use a deterministic ID based on the original op and index
        // to ensure idempotency (same decomposition produces same IDs)
        let decomposed_op_id = generate_decomposed_op_id(&original_op.op_id, index);

        let op_type = match &node_change.kind {
            ReplicatedNodeChangeKind::Upsert => OpType::UpsertNodeSnapshot {
                node: node_change.node.clone(),
                parent_id: node_change.parent_id.clone(),
                revision: branch_head,
                cf_order_key: node_change.cf_order_key.clone(),
            },
            ReplicatedNodeChangeKind::Delete => OpType::DeleteNodeSnapshot {
                node_id: node_change.node.id.clone(),
                revision: branch_head,
            },
        };

        let decomposed_op = Operation {
            op_id: decomposed_op_id,
            op_seq: original_op.op_seq, // Same sequence number (part of same logical operation)
            cluster_node_id: original_op.cluster_node_id.clone(),
            timestamp_ms: original_op.timestamp_ms,
            vector_clock: original_op.vector_clock.clone(),
            tenant_id: original_op.tenant_id.clone(),
            repo_id: original_op.repo_id.clone(),
            branch: original_op.branch.clone(),
            op_type,
            revision: Some(branch_head),
            actor: original_op.actor.clone(),
            message: original_op.message.clone(),
            is_system: original_op.is_system,
            acknowledged_by: HashSet::new(), // Reset acknowledgments for decomposed ops
        };

        decomposed.push(decomposed_op);
    }

    decomposed
}

/// Generate a deterministic operation ID for a decomposed operation
///
/// This ensures that decomposing the same operation multiple times produces
/// the same decomposed operation IDs, which is critical for idempotency.
///
/// We use a simple deterministic hash based on the original op ID and index.
fn generate_decomposed_op_id(original_op_id: &Uuid, index: usize) -> Uuid {
    // Create a deterministic hash from original ID + index
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    original_op_id.hash(&mut hasher);
    index.hash(&mut hasher);
    let hash = hasher.finish();

    // Convert hash to UUID bytes
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&hash.to_le_bytes());
    bytes[8..16].copy_from_slice(&index.to_le_bytes());

    Uuid::from_bytes(bytes)
}

/// Add new OpType variants for decomposed operations
///
/// These should be added to the OpType enum in operation.rs:
///
/// ```ignore
/// /// Upsert a node snapshot (decomposed from ApplyRevision)
/// UpsertNodeSnapshot {
///     node: Node,
///     parent_id: Option<String>,
///     revision: HLC,
/// },
///
/// /// Delete a node snapshot (decomposed from ApplyRevision)
/// DeleteNodeSnapshot {
///     node_id: String,
///     revision: HLC,
/// },
/// ```

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VectorClock;
    use raisin_hlc::HLC;
    use raisin_models::nodes::Node;

    fn make_test_node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            name: "test".to_string(),
            node_type: "Document".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_decompose_non_apply_revision() {
        let op = Operation {
            op_id: Uuid::new_v4(),
            op_seq: 1,
            cluster_node_id: "node1".to_string(),
            timestamp_ms: 1000,
            vector_clock: VectorClock::new(),
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: "test".to_string(),
                property_name: "title".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::String("Test".to_string()),
            },
            revision: None,
            actor: "user".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: Default::default(),
        };

        let decomposed = decompose_operation(op.clone());

        // Should return original operation unchanged
        assert_eq!(decomposed.len(), 1);
        assert_eq!(decomposed[0].op_id, op.op_id);
    }

    #[test]
    fn test_decompose_apply_revision() {
        let mut vc = VectorClock::new();
        vc.set("node1", 10);

        let node_changes = vec![
            ReplicatedNodeChange {
                node: make_test_node("node_a"),
                parent_id: None,
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: String::new(),
            },
            ReplicatedNodeChange {
                node: make_test_node("node_b"),
                parent_id: Some("node_a".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: String::new(),
            },
            ReplicatedNodeChange {
                node: make_test_node("node_c"),
                parent_id: None,
                kind: ReplicatedNodeChangeKind::Delete,
                cf_order_key: String::new(),
            },
        ];

        let branch_head = HLC::new(100, 0);

        let op = Operation {
            op_id: Uuid::new_v4(),
            op_seq: 5,
            cluster_node_id: "node1".to_string(),
            timestamp_ms: 2000,
            vector_clock: vc.clone(),
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::ApplyRevision {
                branch_head,
                node_changes: node_changes.clone(),
            },
            revision: Some(branch_head),
            actor: "user".to_string(),
            message: Some("Test commit".to_string()),
            is_system: false,
            acknowledged_by: Default::default(),
        };

        let decomposed = decompose_operation(op.clone());

        // Should decompose into 3 operations (one per node change)
        assert_eq!(decomposed.len(), 3);

        // Check first decomposed op (Upsert node_a)
        assert_eq!(decomposed[0].tenant_id, "t1");
        assert_eq!(decomposed[0].repo_id, "r1");
        assert_eq!(decomposed[0].branch, "main");
        assert_eq!(decomposed[0].vector_clock, vc);
        assert_eq!(decomposed[0].timestamp_ms, 2000);

        match &decomposed[0].op_type {
            OpType::UpsertNodeSnapshot {
                node,
                parent_id,
                revision,
                cf_order_key: _,
            } => {
                assert_eq!(node.id, "node_a");
                assert_eq!(parent_id, &None);
                assert_eq!(*revision, branch_head);
            }
            _ => panic!("Expected UpsertNodeSnapshot"),
        }

        // Check second decomposed op (Upsert node_b)
        match &decomposed[1].op_type {
            OpType::UpsertNodeSnapshot {
                node,
                parent_id,
                revision,
                cf_order_key: _,
            } => {
                assert_eq!(node.id, "node_b");
                assert_eq!(parent_id, &Some("node_a".to_string()));
                assert_eq!(*revision, branch_head);
            }
            _ => panic!("Expected UpsertNodeSnapshot"),
        }

        // Check third decomposed op (Delete node_c)
        match &decomposed[2].op_type {
            OpType::DeleteNodeSnapshot { node_id, revision } => {
                assert_eq!(node_id, "node_c");
                assert_eq!(*revision, branch_head);
            }
            _ => panic!("Expected DeleteNodeSnapshot"),
        }
    }

    #[test]
    fn test_deterministic_decomposed_ids() {
        let op_id = Uuid::new_v4();

        let id1_a = generate_decomposed_op_id(&op_id, 0);
        let id1_b = generate_decomposed_op_id(&op_id, 0);

        // Same input should produce same ID
        assert_eq!(id1_a, id1_b);

        let id2 = generate_decomposed_op_id(&op_id, 1);

        // Different index should produce different ID
        assert_ne!(id1_a, id2);
    }
}
