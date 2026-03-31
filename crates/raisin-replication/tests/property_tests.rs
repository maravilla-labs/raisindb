//! Property-Based Tests for CRDT Replication
//!
//! These tests verify the correctness of the CRDT replication system
//! by generating random operation sequences and checking that key
//! properties hold.
//!
//! ## CRDT Properties Verified
//!
//! 1. **Strong Eventual Consistency (SEC)**: If two replicas have delivered
//!    the same set of operations, they must have equivalent state.
//!
//! 2. **Idempotency**: Applying the same operation twice has the same effect
//!    as applying it once.
//!
//! 3. **Commutativity**: Concurrent operations can be applied in any order
//!    and converge to the same state.
//!
//! 4. **Causal Consistency**: Operations are delivered in causal order,
//!    preserving happens-before relationships.
//!
//! ## Additional CRDT-Specific Properties
//!
//! 5. **Last-Write-Wins (LWW)**: For property updates, the operation with
//!    the highest vector clock wins.
//!
//! 6. **Add-Wins**: For relations, additions win over concurrent deletions.
//!
//! 7. **Delete-Wins**: For nodes, deletions win over concurrent updates.
//!
//! 8. **RGA Convergence**: List operations converge to consistent order.

use proptest::prelude::*;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::RelationRef;
use raisin_replication::{
    causal_delivery::CausalDeliveryBuffer,
    crdt::{ConflictType, CrdtMerge, MergeResult},
    operation::{OpType, Operation},
    vector_clock::VectorClock,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ============================================================================
// Test Helper Structures
// ============================================================================

/// Simplified replica state for testing CRDT properties
#[derive(Debug, Clone, PartialEq)]
struct ReplicaState {
    /// Node properties (node_id -> property_name -> (value, vector_clock, timestamp, cluster_node_id))
    /// We store vector_clock, timestamp, and cluster_node_id for LWW tie-breaking
    properties: HashMap<String, HashMap<String, (PropertyValue, VectorClock, u64, String)>>,

    /// Relations (source_id -> relation_type -> target_id -> vector_clock)
    relations: HashMap<String, HashMap<String, HashMap<String, VectorClock>>>,

    /// Deleted nodes (node_id -> vector_clock)
    deleted_nodes: HashMap<String, VectorClock>,

    /// List elements (node_id -> list_property -> element_id -> (value, after_id, vector_clock))
    lists:
        HashMap<String, HashMap<String, HashMap<Uuid, (PropertyValue, Option<Uuid>, VectorClock)>>>,
}

impl ReplicaState {
    fn new() -> Self {
        Self {
            properties: HashMap::new(),
            relations: HashMap::new(),
            deleted_nodes: HashMap::new(),
            lists: HashMap::new(),
        }
    }

    /// Apply an operation to this replica state using CRDT merge rules
    fn apply(&mut self, op: &Operation) {
        match &op.op_type {
            OpType::SetProperty {
                node_id,
                property_name,
                value,
            } => {
                self.apply_set_property(
                    node_id,
                    property_name,
                    value,
                    op.timestamp_ms,
                    &op.cluster_node_id,
                    &op.vector_clock,
                );
            }
            OpType::DeleteProperty {
                node_id,
                property_name,
            } => {
                self.apply_delete_property(
                    node_id,
                    property_name,
                    op.timestamp_ms,
                    &op.cluster_node_id,
                    &op.vector_clock,
                );
            }
            OpType::AddRelation {
                source_id,
                relation_type,
                target_id,
                ..
            } => {
                self.apply_add_relation(source_id, relation_type, target_id, &op.vector_clock);
            }
            OpType::RemoveRelation {
                source_id,
                relation_type,
                target_id,
                ..
            } => {
                self.apply_remove_relation(source_id, relation_type, target_id, &op.vector_clock);
            }
            OpType::DeleteNode { node_id } => {
                self.apply_delete_node(node_id, &op.vector_clock);
            }
            OpType::ListInsertAfter {
                node_id,
                list_property,
                after_id,
                value,
                element_id,
            } => {
                self.apply_list_insert(
                    node_id,
                    list_property,
                    *element_id,
                    value,
                    *after_id,
                    &op.vector_clock,
                );
            }
            OpType::ListDelete {
                node_id,
                list_property,
                element_id,
            } => {
                self.apply_list_delete(node_id, list_property, *element_id, &op.vector_clock);
            }
            _ => {
                // For other operation types, we don't track state in this simplified model
            }
        }
    }

    fn apply_set_property(
        &mut self,
        node_id: &str,
        property_name: &str,
        value: &PropertyValue,
        timestamp: u64,
        cluster_node_id: &str,
        vc: &VectorClock,
    ) {
        let node_props = self
            .properties
            .entry(node_id.to_string())
            .or_insert_with(HashMap::new);

        // LWW with three-level tie-breaking (matching CrdtMerge::compare_operations_lww):
        // 1. Vector clock (causal ordering)
        // 2. Timestamp (wall clock)
        // 3. Cluster node ID (deterministic)
        if let Some((_, existing_vc, existing_timestamp, existing_cluster_node_id)) =
            node_props.get(property_name)
        {
            let should_update = if vc.happens_after(existing_vc) {
                true
            } else if vc.happens_before(existing_vc) {
                false
            } else {
                // Concurrent or equal - use timestamp tie-breaker
                if timestamp > *existing_timestamp {
                    true
                } else if timestamp < *existing_timestamp {
                    false
                } else {
                    // Same timestamp - use cluster node ID as final tie-breaker
                    cluster_node_id > existing_cluster_node_id.as_str()
                }
            };

            if should_update {
                node_props.insert(
                    property_name.to_string(),
                    (
                        value.clone(),
                        vc.clone(),
                        timestamp,
                        cluster_node_id.to_string(),
                    ),
                );
            }
        } else {
            node_props.insert(
                property_name.to_string(),
                (
                    value.clone(),
                    vc.clone(),
                    timestamp,
                    cluster_node_id.to_string(),
                ),
            );
        }
    }

    fn apply_delete_property(
        &mut self,
        node_id: &str,
        property_name: &str,
        timestamp: u64,
        cluster_node_id: &str,
        vc: &VectorClock,
    ) {
        if let Some(node_props) = self.properties.get_mut(node_id) {
            if let Some((_, existing_vc, existing_timestamp, existing_cluster_node_id)) =
                node_props.get(property_name)
            {
                // Same LWW logic as set_property
                let should_delete = if vc.happens_after(existing_vc) {
                    true
                } else if vc.happens_before(existing_vc) {
                    false
                } else {
                    if timestamp > *existing_timestamp {
                        true
                    } else if timestamp < *existing_timestamp {
                        false
                    } else {
                        cluster_node_id > existing_cluster_node_id.as_str()
                    }
                };

                if should_delete {
                    node_props.remove(property_name);
                }
            }
        }
    }

    fn apply_add_relation(
        &mut self,
        source_id: &str,
        relation_type: &str,
        target_id: &str,
        vc: &VectorClock,
    ) {
        let source_relations = self
            .relations
            .entry(source_id.to_string())
            .or_insert_with(HashMap::new);
        let typed_relations = source_relations
            .entry(relation_type.to_string())
            .or_insert_with(HashMap::new);

        // Add-Wins: Store the relation with its vector clock
        typed_relations.insert(target_id.to_string(), vc.clone());
    }

    fn apply_remove_relation(
        &mut self,
        source_id: &str,
        relation_type: &str,
        target_id: &str,
        vc: &VectorClock,
    ) {
        if let Some(source_relations) = self.relations.get_mut(source_id) {
            if let Some(typed_relations) = source_relations.get_mut(relation_type) {
                if let Some(add_vc) = typed_relations.get(target_id) {
                    // Add-Wins: Only remove if delete happened after add
                    if vc.happens_after(add_vc) {
                        typed_relations.remove(target_id);
                    }
                    // If concurrent or before, keep the relation (Add-Wins)
                }
            }
        }
    }

    fn apply_delete_node(&mut self, node_id: &str, vc: &VectorClock) {
        // Delete-Wins: Mark node as deleted
        if let Some(existing_vc) = self.deleted_nodes.get(node_id) {
            if vc.happens_after(existing_vc) {
                self.deleted_nodes.insert(node_id.to_string(), vc.clone());
            }
        } else {
            self.deleted_nodes.insert(node_id.to_string(), vc.clone());
        }
    }

    fn apply_list_insert(
        &mut self,
        node_id: &str,
        list_property: &str,
        element_id: Uuid,
        value: &PropertyValue,
        after_id: Option<Uuid>,
        vc: &VectorClock,
    ) {
        let node_lists = self
            .lists
            .entry(node_id.to_string())
            .or_insert_with(HashMap::new);
        let list = node_lists
            .entry(list_property.to_string())
            .or_insert_with(HashMap::new);

        // RGA: Insert element if not already present
        list.entry(element_id)
            .or_insert((value.clone(), after_id, vc.clone()));
    }

    fn apply_list_delete(
        &mut self,
        node_id: &str,
        list_property: &str,
        element_id: Uuid,
        vc: &VectorClock,
    ) {
        if let Some(node_lists) = self.lists.get_mut(node_id) {
            if let Some(list) = node_lists.get_mut(list_property) {
                if let Some((_, _, insert_vc)) = list.get(&element_id) {
                    // RGA: Only delete if delete happened after insert
                    if vc.happens_after(insert_vc) {
                        list.remove(&element_id);
                    }
                }
            }
        }
    }
}

/// Check if two replicas have equivalent state
fn replicas_equivalent(r1: &ReplicaState, r2: &ReplicaState) -> bool {
    r1.properties == r2.properties
        && r1.relations == r2.relations
        && r1.deleted_nodes == r2.deleted_nodes
        && r1.lists == r2.lists
}

// ============================================================================
// Property Generators
// ============================================================================

/// Generate a random node ID
fn arb_node_id() -> impl Strategy<Value = String> {
    prop::string::string_regex("node[0-9]{1,3}").unwrap()
}

/// Generate a random property name
fn arb_property_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("title".to_string()),
        Just("content".to_string()),
        Just("count".to_string()),
        Just("status".to_string()),
    ]
}

/// Generate a random property value
fn arb_property_value() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        any::<String>().prop_map(PropertyValue::String),
        any::<f64>()
            .prop_filter("Must be finite", |x| x.is_finite())
            .prop_map(PropertyValue::Float),
        any::<bool>().prop_map(PropertyValue::Boolean),
    ]
}

/// Generate a random cluster node ID
fn arb_cluster_node_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("node1".to_string()),
        Just("node2".to_string()),
        Just("node3".to_string()),
    ]
}

/// Generate a SetProperty operation from a specific cluster node
fn arb_set_property_from_node(
    cluster_node: String,
    vc_counter: u64,
) -> impl Strategy<Value = Operation> {
    (
        arb_node_id(),
        arb_property_name(),
        arb_property_value(),
        any::<u64>(),
    )
        .prop_map(move |(node_id, prop_name, value, timestamp)| {
            let mut vc = VectorClock::new();
            vc.set(&cluster_node, vc_counter);

            Operation {
                op_id: Uuid::new_v4(),
                op_seq: vc_counter,
                cluster_node_id: cluster_node.clone(),
                timestamp_ms: timestamp % 1_000_000,
                vector_clock: vc,
                tenant_id: "tenant1".to_string(),
                repo_id: "repo1".to_string(),
                branch: "main".to_string(),
                op_type: OpType::SetProperty {
                    node_id,
                    property_name: prop_name,
                    value,
                },
                revision: None,
                actor: "test".to_string(),
                message: None,
                is_system: false,
                acknowledged_by: HashSet::new(),
            }
        })
}

/// Generate a DeleteProperty operation
fn arb_delete_property_from_node(
    cluster_node: String,
    vc_counter: u64,
) -> impl Strategy<Value = Operation> {
    (arb_node_id(), arb_property_name(), any::<u64>()).prop_map(
        move |(node_id, prop_name, timestamp)| {
            let mut vc = VectorClock::new();
            vc.set(&cluster_node, vc_counter);

            Operation {
                op_id: Uuid::new_v4(),
                op_seq: vc_counter,
                cluster_node_id: cluster_node.clone(),
                timestamp_ms: timestamp % 1_000_000,
                vector_clock: vc,
                tenant_id: "tenant1".to_string(),
                repo_id: "repo1".to_string(),
                branch: "main".to_string(),
                op_type: OpType::DeleteProperty {
                    node_id,
                    property_name: prop_name,
                },
                revision: None,
                actor: "test".to_string(),
                message: None,
                is_system: false,
                acknowledged_by: HashSet::new(),
            }
        },
    )
}

/// Generate an AddRelation operation
fn arb_add_relation_from_node(
    cluster_node: String,
    vc_counter: u64,
) -> impl Strategy<Value = Operation> {
    (arb_node_id(), arb_node_id(), any::<u64>()).prop_map(
        move |(source_id, target_id, timestamp)| {
            let mut vc = VectorClock::new();
            vc.set(&cluster_node, vc_counter);

            Operation {
                op_id: Uuid::new_v4(),
                op_seq: vc_counter,
                cluster_node_id: cluster_node.clone(),
                timestamp_ms: timestamp % 1_000_000,
                vector_clock: vc,
                tenant_id: "tenant1".to_string(),
                repo_id: "repo1".to_string(),
                branch: "main".to_string(),
                op_type: OpType::AddRelation {
                    source_id,
                    source_workspace: "workspace".to_string(),
                    relation_type: "refs".to_string(),
                    target_id: target_id.clone(),
                    target_workspace: "workspace".to_string(),
                    relation: RelationRef::new(
                        target_id,
                        "workspace".to_string(),
                        "".to_string(),
                        "refs".to_string(),
                        None,
                    ),
                },
                revision: None,
                actor: "test".to_string(),
                message: None,
                is_system: false,
                acknowledged_by: HashSet::new(),
            }
        },
    )
}

/// Generate a RemoveRelation operation
fn arb_remove_relation_from_node(
    cluster_node: String,
    vc_counter: u64,
    source_id: &str,
    target_id: &str,
    relation_type: &str,
) -> Operation {
    let mut vc = VectorClock::new();
    vc.set(&cluster_node, vc_counter);

    Operation {
        op_id: Uuid::new_v4(),
        op_seq: vc_counter,
        cluster_node_id: cluster_node.clone(),
        timestamp_ms: 2000,
        vector_clock: vc,
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::RemoveRelation {
            source_id: source_id.to_string(),
            source_workspace: "workspace".to_string(),
            relation_type: relation_type.to_string(),
            target_id: target_id.to_string(),
            target_workspace: "workspace".to_string(),
        },
        revision: None,
        actor: "test".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    }
}

/// Generate a DeleteNode operation
fn arb_delete_node_from_node(
    cluster_node: String,
    vc_counter: u64,
) -> impl Strategy<Value = Operation> {
    (arb_node_id(), any::<u64>()).prop_map(move |(node_id, timestamp)| {
        let mut vc = VectorClock::new();
        vc.set(&cluster_node, vc_counter);

        Operation {
            op_id: Uuid::new_v4(),
            op_seq: vc_counter,
            cluster_node_id: cluster_node.clone(),
            timestamp_ms: timestamp % 1_000_000,
            vector_clock: vc,
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::DeleteNode { node_id },
            revision: None,
            actor: "test".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: HashSet::new(),
        }
    })
}

// Note: We don't need a complex operation sequence generator for these tests
// Instead, we'll generate operations directly in the proptest macros

// ============================================================================
// Property Tests
// ============================================================================

// Property 1: Strong Eventual Consistency
// If two replicas have delivered the same set of operations, they must have equivalent state

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_strong_eventual_consistency(
        ops1 in prop::collection::vec(arb_set_property_from_node("node1".to_string(), 1), 1..10),
        ops2 in prop::collection::vec(arb_set_property_from_node("node2".to_string(), 1), 1..10),
    ) {
        let mut replica_a = ReplicaState::new();
        let mut replica_b = ReplicaState::new();

        // Replica A: apply ops1 then ops2
        for op in &ops1 { replica_a.apply(op); }
        for op in &ops2 { replica_a.apply(op); }

        // Replica B: apply ops2 then ops1 (reverse order)
        for op in &ops2 { replica_b.apply(op); }
        for op in &ops1 { replica_b.apply(op); }

        // Both replicas should converge to equivalent state
        prop_assert!(
            replicas_equivalent(&replica_a, &replica_b),
            "Replicas did not converge despite receiving same operations"
        );
    }
}

// Property 2: Idempotency
// Applying the same operation twice has the same effect as applying it once

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_idempotency_set_property(
        ops in prop::collection::vec(arb_set_property_from_node("node1".to_string(), 1), 1..10)
    ) {
        let mut replica1 = ReplicaState::new();
        let mut replica2 = ReplicaState::new();

        // Replica 1: apply each operation once
        for op in &ops {
            replica1.apply(op);
        }

        // Replica 2: apply each operation twice
        for op in &ops {
            replica2.apply(op);
            replica2.apply(op);
        }

        prop_assert!(
            replicas_equivalent(&replica1, &replica2),
            "Idempotency violated: applying twice gave different result than once"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_idempotency_add_relation(
        ops in prop::collection::vec(arb_add_relation_from_node("node1".to_string(), 1), 1..10)
    ) {
        let mut replica1 = ReplicaState::new();
        let mut replica2 = ReplicaState::new();

        for op in &ops {
            replica1.apply(op);
        }

        for op in &ops {
            replica2.apply(op);
            replica2.apply(op);
        }

        prop_assert!(
            replicas_equivalent(&replica1, &replica2),
            "Add relation idempotency violated"
        );
    }
}

// Property 3: Commutativity
// Concurrent operations can be applied in any order

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_commutativity_concurrent_ops(
        ops1 in prop::collection::vec(arb_set_property_from_node("node1".to_string(), 1), 1..8),
        ops2 in prop::collection::vec(arb_set_property_from_node("node2".to_string(), 1), 1..8),
    ) {
        let mut replica_a = ReplicaState::new();
        let mut replica_b = ReplicaState::new();

        // Replica A: ops1 then ops2
        for op in &ops1 { replica_a.apply(op); }
        for op in &ops2 { replica_a.apply(op); }

        // Replica B: ops2 then ops1
        for op in &ops2 { replica_b.apply(op); }
        for op in &ops1 { replica_b.apply(op); }

        prop_assert!(
            replicas_equivalent(&replica_a, &replica_b),
            "Commutativity violated: different order gave different state"
        );
    }
}

// Property 4: Causal Delivery Ensures Convergence
// Operations delivered in causal order maintain happens-before relationships

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    #[test]
    fn prop_causal_delivery_preserves_order(
        seed in any::<u64>(),
    ) {
        use rand::{SeedableRng, seq::SliceRandom};
        use rand::rngs::StdRng;

        let mut rng = StdRng::seed_from_u64(seed);

        // Create a sequence of causally ordered operations
        let mut ops = Vec::new();
        let mut vc = VectorClock::new();

        for i in 1..=10 {
            vc.increment("node1");

            ops.push(Operation {
                op_id: Uuid::new_v4(),
                op_seq: i,
                cluster_node_id: "node1".to_string(),
                timestamp_ms: 1000 * i,
                vector_clock: vc.clone(),
                tenant_id: "tenant1".to_string(),
                repo_id: "repo1".to_string(),
                branch: "main".to_string(),
                op_type: OpType::SetProperty {
                    node_id: "test".to_string(),
                    property_name: "value".to_string(),
                    value: PropertyValue::Integer(i as i64),
                },
                revision: None,
                actor: "test".to_string(),
                message: None,
                is_system: false,
                acknowledged_by: HashSet::new(),
            });
        }

        // Shuffle operations
        let mut shuffled = ops.clone();
        shuffled.shuffle(&mut rng);

        // Deliver through causal buffer
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
        let mut delivered = Vec::new();

        for op in shuffled {
            let mut batch = buffer.deliver(op);
            delivered.append(&mut batch);
        }

        // All operations should eventually be delivered
        prop_assert_eq!(
            delivered.len(),
            ops.len(),
            "Not all operations were delivered"
        );

        // Verify causal order is preserved
        for i in 0..delivered.len() {
            for j in (i+1)..delivered.len() {
                let op1 = &delivered[i];
                let op2 = &delivered[j];

                // If op2 causally depends on op1, op1 should come first
                if op1.vector_clock.happens_before(&op2.vector_clock) {
                    // Already in correct order - good!
                } else if op2.vector_clock.happens_before(&op1.vector_clock) {
                    prop_assert!(
                        false,
                        "Causal order violated: op at index {} happened after op at index {}",
                        i, j
                    );
                }
            }
        }
    }
}

// Property 5: Last-Write-Wins for Property Updates
// The operation with the highest vector clock wins

#[test]
fn prop_lww_property_updates() {
    // Create two concurrent operations with different timestamps
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 1);

    let mut vc2 = VectorClock::new();
    vc2.set("node2", 1);

    let op1 = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node1".to_string(),
        timestamp_ms: 1000,
        vector_clock: vc1,
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "test".to_string(),
            property_name: "title".to_string(),
            value: PropertyValue::String("Value 1".to_string()),
        },
        revision: None,
        actor: "user1".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    };

    let op2 = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node2".to_string(),
        timestamp_ms: 2000, // Later timestamp
        vector_clock: vc2,
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "test".to_string(),
            property_name: "title".to_string(),
            value: PropertyValue::String("Value 2".to_string()),
        },
        revision: None,
        actor: "user2".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    };

    // Merge operations
    let result = CrdtMerge::merge_operations(vec![op1.clone(), op2.clone()]);

    match result {
        MergeResult::Conflict {
            winner,
            conflict_type,
            ..
        } => {
            assert_eq!(conflict_type, ConflictType::ConcurrentPropertyUpdate);
            // op2 should win (later timestamp)
            assert_eq!(winner.op_id, op2.op_id);
        }
        MergeResult::Winner(winner) => {
            // If not detected as conflict, should still be op2
            assert_eq!(winner.op_id, op2.op_id);
        }
    }
}

// Property 6: Add-Wins for Relations
// Additions win over concurrent deletions

#[test]
fn prop_add_wins_relations() {
    // Concurrent add and remove
    let mut vc_add = VectorClock::new();
    vc_add.set("node1", 1);

    let mut vc_remove = VectorClock::new();
    vc_remove.set("node2", 1);

    let add_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node1".to_string(),
        timestamp_ms: 1000,
        vector_clock: vc_add.clone(),
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

    let remove_op =
        arb_remove_relation_from_node("node2".to_string(), 1, "source", "target", "refs");

    // Apply in both orders
    let mut replica1 = ReplicaState::new();
    replica1.apply(&add_op);
    replica1.apply(&remove_op);

    let mut replica2 = ReplicaState::new();
    replica2.apply(&remove_op);
    replica2.apply(&add_op);

    // Both should have the relation (Add-Wins)
    assert_eq!(
        replica1
            .relations
            .get("source")
            .unwrap()
            .get("refs")
            .unwrap()
            .get("target")
            .is_some(),
        true
    );
    assert_eq!(
        replica2
            .relations
            .get("source")
            .unwrap()
            .get("refs")
            .unwrap()
            .get("target")
            .is_some(),
        true
    );
    assert!(replicas_equivalent(&replica1, &replica2));
}

// Property 7: Delete-Wins for Nodes
// Node deletions win over concurrent updates

#[test]
fn prop_delete_wins_nodes() {
    let node_id = "test_node";

    // Concurrent property update and node delete
    let mut vc_update = VectorClock::new();
    vc_update.set("node1", 1);

    let mut vc_delete = VectorClock::new();
    vc_delete.set("node2", 1);

    let update_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node1".to_string(),
        timestamp_ms: 1000,
        vector_clock: vc_update,
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: node_id.to_string(),
            property_name: "title".to_string(),
            value: PropertyValue::String("Updated".to_string()),
        },
        revision: None,
        actor: "user".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    };

    let delete_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node2".to_string(),
        timestamp_ms: 1000,
        vector_clock: vc_delete,
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::DeleteNode {
            node_id: node_id.to_string(),
        },
        revision: None,
        actor: "user".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    };

    // Apply in both orders
    let mut replica1 = ReplicaState::new();
    replica1.apply(&update_op);
    replica1.apply(&delete_op);

    let mut replica2 = ReplicaState::new();
    replica2.apply(&delete_op);
    replica2.apply(&update_op);

    // Both should have the node deleted
    assert!(replica1.deleted_nodes.contains_key(node_id));
    assert!(replica2.deleted_nodes.contains_key(node_id));
    assert!(replicas_equivalent(&replica1, &replica2));
}

// Property 8: Vector Clock Ordering Properties

#[test]
fn prop_vector_clock_transitivity() {
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 1);

    let mut vc2 = VectorClock::new();
    vc2.set("node1", 2);

    let mut vc3 = VectorClock::new();
    vc3.set("node1", 3);

    // Transitivity: if vc1 < vc2 and vc2 < vc3, then vc1 < vc3
    assert!(vc1.happens_before(&vc2));
    assert!(vc2.happens_before(&vc3));
    assert!(vc1.happens_before(&vc3));
}

#[test]
fn prop_vector_clock_concurrent_symmetry() {
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 1);

    let mut vc2 = VectorClock::new();
    vc2.set("node2", 1);

    // Symmetry: if vc1 || vc2 (concurrent), then vc2 || vc1
    assert!(vc1.concurrent_with(&vc2));
    assert!(vc2.concurrent_with(&vc1));
}

#[test]
fn prop_vector_clock_merge_idempotency() {
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 5);
    vc1.set("node2", 3);

    let vc2 = vc1.clone();

    let merged = vc1.clone().merged(&vc2);

    // Merging with itself should be idempotent
    assert_eq!(merged, vc1);
}

#[test]
fn prop_vector_clock_merge_commutativity() {
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 5);
    vc1.set("node2", 3);

    let mut vc2 = VectorClock::new();
    vc2.set("node1", 3);
    vc2.set("node2", 7);
    vc2.set("node3", 2);

    let merged_1_2 = vc1.clone().merged(&vc2);
    let merged_2_1 = vc2.clone().merged(&vc1);

    // Merge should be commutative
    assert_eq!(merged_1_2, merged_2_1);
}

// Property 9: CRDT Merge Rules

#[test]
fn prop_crdt_merge_deterministic() {
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 1);

    let mut vc2 = VectorClock::new();
    vc2.set("node2", 1);

    let op1 = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node1".to_string(),
        timestamp_ms: 1000,
        vector_clock: vc1,
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "test".to_string(),
            property_name: "title".to_string(),
            value: PropertyValue::String("Value 1".to_string()),
        },
        revision: None,
        actor: "user".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    };

    let op2 = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node2".to_string(),
        timestamp_ms: 2000,
        vector_clock: vc2,
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "test".to_string(),
            property_name: "title".to_string(),
            value: PropertyValue::String("Value 2".to_string()),
        },
        revision: None,
        actor: "user".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: HashSet::new(),
    };

    // Merge in both orders
    let result1 = CrdtMerge::merge_operations(vec![op1.clone(), op2.clone()]);
    let result2 = CrdtMerge::merge_operations(vec![op2.clone(), op1.clone()]);

    // Should produce same winner regardless of input order
    let winner1_id = match result1 {
        MergeResult::Winner(op) => op.op_id,
        MergeResult::Conflict { winner, .. } => winner.op_id,
    };

    let winner2_id = match result2 {
        MergeResult::Winner(op) => op.op_id,
        MergeResult::Conflict { winner, .. } => winner.op_id,
    };

    assert_eq!(winner1_id, winner2_id, "Merge should be deterministic");
}

// Property 10: Causal Buffer Completeness
// All operations are eventually delivered

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    #[test]
    fn prop_causal_buffer_completeness(
        seed in any::<u64>(),
    ) {
        use rand::{SeedableRng, seq::SliceRandom};
        use rand::rngs::StdRng;

        let mut rng = StdRng::seed_from_u64(seed);

        // Create operations from multiple nodes
        let mut all_ops = Vec::new();
        let mut vc = VectorClock::new();

        for node in ["node1", "node2", "node3"] {
            for i in 1..=5 {
                vc.increment(node);

                all_ops.push(Operation {
                    op_id: Uuid::new_v4(),
                    op_seq: i,
                    cluster_node_id: node.to_string(),
                    timestamp_ms: 1000 * i,
                    vector_clock: vc.clone(),
                    tenant_id: "tenant1".to_string(),
                    repo_id: "repo1".to_string(),
                    branch: "main".to_string(),
                    op_type: OpType::SetProperty {
                        node_id: format!("{}_data", node),
                        property_name: "value".to_string(),
                        value: PropertyValue::Integer(i as i64),
                    },
                    revision: None,
                    actor: "test".to_string(),
                    message: None,
                    is_system: false,
                    acknowledged_by: HashSet::new(),
                });
            }
        }

        // Shuffle operations
        let mut shuffled = all_ops.clone();
        shuffled.shuffle(&mut rng);

        // Deliver through causal buffer
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);
        let mut delivered = Vec::new();

        for op in shuffled {
            let mut batch = buffer.deliver(op);
            delivered.append(&mut batch);
        }

        // All operations should eventually be delivered
        prop_assert_eq!(
            delivered.len(),
            all_ops.len(),
            "Causal buffer did not deliver all operations"
        );

        // Buffer should be empty after all deliveries
        prop_assert_eq!(
            buffer.buffer_size(),
            0,
            "Causal buffer should be empty after delivering all operations"
        );
    }
}
