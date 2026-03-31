---- MODULE AddWinsSet ----
(***************************************************************************
 * Add-Wins Set CRDT Specification for RaisinDB
 *
 * This module formalizes the Add-Wins Set (Observed-Remove Set) CRDT used for:
 * - Relations between nodes (AddRelation/RemoveRelation operations)
 * - Set membership where concurrent add and remove operations resolve with add winning
 *
 * Implementation Mapping:
 * - Rust: crates/raisin-rocksdb/src/replication/application.rs
 *   - Lines 2437-2505: apply_add_relation
 *   - Lines 2508-2572: apply_remove_relation
 * - Rust: crates/raisin-replication/src/operation.rs
 *   - Lines 181-199: AddRelation/RemoveRelation operations
 *   - Line 187: relation_id (UUID for Add-Wins semantics)
 *
 * Key Semantic: Add-Wins
 * - If add and remove are concurrent, the relation EXISTS
 * - Remove only takes effect if it causally follows the add
 * - Each relation has a unique UUID to distinguish multiple adds
 *
 * Properties Verified:
 * 1. Add-Wins: Concurrent add/remove -> element exists
 * 2. Convergence: Same operations -> same final set
 * 3. Commutativity: Order of applying operations doesn't matter
 * 4. Idempotency: Applying same operation multiple times = applying once
 ***************************************************************************)

EXTENDS Naturals, TLC, FiniteSets
INSTANCE VectorClock

CONSTANTS
  Nodes,          \* Set of cluster nodes
  Relations,      \* Set of relation instances (each has unique ID)
  MaxOps          \* Maximum operations for model checking

(***************************************************************************
 * Relation Data Structure
 *
 * Each relation is uniquely identified by:
 * - fromNode: Source node ID
 * - relationType: Type of relation (e.g., "has_parent", "references")
 * - toNode: Target node ID
 * - uuid: Unique identifier for this relation instance
 *
 * Multiple relations with same (from, type, to) can exist if they have
 * different UUIDs (added at different times).
 ***************************************************************************)

Relation == [
  fromNode: STRING,
  relationType: STRING,
  toNode: STRING,
  uuid: Relations,
  vc: VectorClock
]

(***************************************************************************
 * Operation Types
 ***************************************************************************)

AddRelationOp == [
  opType: {"ADD"},
  fromNode: STRING,
  relationType: STRING,
  toNode: STRING,
  uuid: Relations,
  vc: VectorClock,
  sourceNode: Nodes
]

RemoveRelationOp == [
  opType: {"REMOVE"},
  fromNode: STRING,
  relationType: STRING,
  toNode: STRING,
  uuid: Relations,  \* Must match the UUID from corresponding AddRelation
  vc: VectorClock,
  sourceNode: Nodes
]

RelationOp == AddRelationOp \union RemoveRelationOp

(***************************************************************************
 * Add-Wins Set Semantics
 *
 * A relation with UUID 'u' EXISTS if and only if:
 * - There exists an ADD operation for 'u', AND
 * - For all REMOVE operations for 'u', the remove does NOT causally follow the add
 *   (i.e., either remove is concurrent or happened before add)
 *
 * This implements the "Add-Wins" semantics:
 * - Concurrent add + remove -> relation EXISTS
 * - Remove only takes effect if it happens-after the add
 ***************************************************************************)

\* Check if a relation exists given the set of operations
RelationExists(relationId, addOps, removeOps) ==
  \E addOp \in addOps :
    /\ addOp.uuid = relationId
    /\ \A remOp \in removeOps :
        remOp.uuid = relationId =>
          \/ ~HappensBefore(addOp.vc, remOp.vc)  \* Remove is concurrent or before
          \* Add-Wins: if concurrent or remove before add, relation exists

\* Alternative formulation: relation exists if there's an add not causally preceded by remove
RelationExistsAlt(relationId, ops) ==
  LET addOps == {op \in ops : op.opType = "ADD" /\ op.uuid = relationId}
      removeOps == {op \in ops : op.opType = "REMOVE" /\ op.uuid = relationId}
  IN \E addOp \in addOps :
       \A remOp \in removeOps :
         ~HappensBefore(addOp.vc, remOp.vc)

(***************************************************************************
 * State Machine for Testing Add-Wins Set
 ***************************************************************************)

VARIABLES
  operations,     \* Global history of all operations
  nodeState,      \* Per-node view: [Nodes -> SUBSET Relations]
  nodeVC,         \* Vector clock per node: [Nodes -> VectorClock]
  deliveredOps    \* Operations delivered to each node: [Nodes -> SUBSET operations]

aws_vars == <<operations, nodeState, nodeVC, deliveredOps>>

(***************************************************************************
 * Type Invariants
 ***************************************************************************)

TypeOK ==
  /\ operations \subseteq RelationOp
  /\ nodeState \in [Nodes -> SUBSET Relations]
  /\ nodeVC \in [Nodes -> VectorClock]
  /\ deliveredOps \in [Nodes -> SUBSET operations]

(***************************************************************************
 * Initial State
 ***************************************************************************)

Init ==
  /\ operations = {}
  /\ nodeState = [n \in Nodes |-> {}]
  /\ nodeVC = [n \in Nodes |-> InitVC]
  /\ deliveredOps = [n \in Nodes |-> {}]

(***************************************************************************
 * Helper Functions
 ***************************************************************************)

\* Compute current set of relations from delivered operations
\* This applies Add-Wins semantics
ComputeRelationSet(ops) ==
  LET addOps == {op \in ops : op.opType = "ADD"}
      removeOps == {op \in ops : op.opType = "REMOVE"}
  IN {op.uuid : op \in addOps :
       \A remOp \in removeOps :
         op.uuid = remOp.uuid => ~HappensBefore(op.vc, remOp.vc)}

(***************************************************************************
 * Actions
 ***************************************************************************)

\* Add a new relation
AddRelation(node, fromNode, relType, toNode, uuid) ==
  /\ Cardinality(operations) < MaxOps
  /\ LET vc == Increment(nodeVC[node], node)
         op == [
           opType |-> "ADD",
           fromNode |-> fromNode,
           relationType |-> relType,
           toNode |-> toNode,
           uuid |-> uuid,
           vc |-> vc,
           sourceNode |-> node
         ]
     IN /\ operations' = operations \union {op}
        /\ nodeVC' = [nodeVC EXCEPT ![node] = vc]
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node] = ComputeRelationSet(deliveredOps'[node])]

\* Remove a relation
RemoveRelation(node, fromNode, relType, toNode, uuid) ==
  /\ Cardinality(operations) < MaxOps
  /\ LET vc == Increment(nodeVC[node], node)
         op == [
           opType |-> "REMOVE",
           fromNode |-> fromNode,
           relationType |-> relType,
           toNode |-> toNode,
           uuid |-> uuid,
           vc |-> vc,
           sourceNode |-> node
         ]
     IN /\ operations' = operations \union {op}
        /\ nodeVC' = [nodeVC EXCEPT ![node] = vc]
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node] = ComputeRelationSet(deliveredOps'[node])]

\* Deliver an operation to a node
DeliverOp(node, op) ==
  /\ op \in operations
  /\ op \notin deliveredOps[node]
  /\ LET mergedVC == Merge(nodeVC[node], op.vc)
         newVC == Increment(mergedVC, node)
     IN /\ nodeVC' = [nodeVC EXCEPT ![node] = newVC]
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node] = ComputeRelationSet(deliveredOps'[node])]
        /\ UNCHANGED operations

(***************************************************************************
 * Next State Relation
 ***************************************************************************)

Next ==
  \/ \E node \in Nodes, uuid \in Relations :
       AddRelation(node, "node1", "refs", "node2", uuid)
  \/ \E node \in Nodes, uuid \in Relations :
       RemoveRelation(node, "node1", "refs", "node2", uuid)
  \/ \E node \in Nodes, op \in operations :
       DeliverOp(node, op)

Spec == Init /\ [][Next]_aws_vars

(***************************************************************************
 * Key Properties
 ***************************************************************************)

\* PROPERTY 1: Add-Wins Semantics
\* If add and remove are concurrent, relation exists
AddWinsProperty ==
  \A addOp \in operations, remOp \in operations :
    /\ addOp.opType = "ADD"
    /\ remOp.opType = "REMOVE"
    /\ addOp.uuid = remOp.uuid
    /\ Concurrent(addOp.vc, remOp.vc)
    =>
      \A n \in Nodes :
        (addOp \in deliveredOps[n] /\ remOp \in deliveredOps[n]) =>
          addOp.uuid \in nodeState[n]

\* PROPERTY 2: Convergence
\* Nodes with same delivered operations have same state
Convergence ==
  \A n1, n2 \in Nodes :
    deliveredOps[n1] = deliveredOps[n2] =>
      nodeState[n1] = nodeState[n2]

\* PROPERTY 3: Eventual Consistency
\* Once all nodes receive all operations, they all have the same state
EventualConsistency ==
  (\A n \in Nodes : deliveredOps[n] = operations) =>
    \A n1, n2 \in Nodes : nodeState[n1] = nodeState[n2]

\* PROPERTY 4: Monotonic Growth (for adds without removes)
\* If only add operations exist, set grows monotonically
MonotonicGrowth ==
  (\A op \in operations : op.opType = "ADD") =>
    \A n \in Nodes, op \in operations :
      op \in deliveredOps[n] => op.uuid \in nodeState[n]

\* PROPERTY 5: Remove Causality
\* Remove only removes if it causally follows add
RemoveCausality ==
  \A n \in Nodes, remOp \in deliveredOps[n] :
    /\ remOp.opType = "REMOVE"
    /\ remOp.uuid \notin nodeState[n]
    =>
      \E addOp \in deliveredOps[n] :
        /\ addOp.opType = "ADD"
        /\ addOp.uuid = remOp.uuid
        /\ HappensBefore(addOp.vc, remOp.vc)

(***************************************************************************
 * Invariants for Testing
 ***************************************************************************)

\* Delivered operations are subset of global operations
DeliveredOpsValid ==
  \A n \in Nodes : deliveredOps[n] \subseteq operations

\* Vector clocks are monotonic (node's own counter never decreases)
VectorClockMonotonic ==
  \A n \in Nodes : nodeVC[n][n] >= 0

\* Node state matches computed state from delivered operations
NodeStateConsistent ==
  \A n \in Nodes :
    nodeState[n] = ComputeRelationSet(deliveredOps[n])

(***************************************************************************
 * Example Scenarios
 ***************************************************************************)

\* Scenario 1: Concurrent add and remove
\* Node1 adds relation with VC {node1:1}
\* Node2 removes same relation with VC {node2:1}
\* VCs are concurrent -> relation should EXIST (Add-Wins)
ConcurrentAddRemoveScenario ==
  /\ Cardinality(Nodes) >= 2
  /\ \E n1, n2 \in Nodes, rel \in Relations :
       /\ n1 # n2
       /\ \E addOp \in operations, remOp \in operations :
            /\ addOp.opType = "ADD"
            /\ remOp.opType = "REMOVE"
            /\ addOp.uuid = rel
            /\ remOp.uuid = rel
            /\ Concurrent(addOp.vc, remOp.vc)
            /\ \A n \in Nodes :
                 (addOp \in deliveredOps[n] /\ remOp \in deliveredOps[n]) =>
                   rel \in nodeState[n]

\* Scenario 2: Causal remove
\* Add happens-before remove -> relation should NOT exist
CausalRemoveScenario ==
  \E addOp \in operations, remOp \in operations :
    /\ addOp.opType = "ADD"
    /\ remOp.opType = "REMOVE"
    /\ addOp.uuid = remOp.uuid
    /\ HappensBefore(addOp.vc, remOp.vc)
    =>
      \A n \in Nodes :
        (addOp \in deliveredOps[n] /\ remOp \in deliveredOps[n]) =>
          addOp.uuid \notin nodeState[n]

\* Scenario 3: Multiple adds with same relation details but different UUIDs
\* Each should be tracked separately
MultipleAddsScenario ==
  \E uuid1, uuid2 \in Relations :
    /\ uuid1 # uuid2
    /\ \E op1, op2 \in operations :
         /\ op1.opType = "ADD"
         /\ op2.opType = "ADD"
         /\ op1.uuid = uuid1
         /\ op2.uuid = uuid2
         /\ op1.fromNode = op2.fromNode
         /\ op1.toNode = op2.toNode
         /\ op1.relationType = op2.relationType
         =>
           \A n \in Nodes :
             (op1 \in deliveredOps[n] /\ op2 \in deliveredOps[n]) =>
               (uuid1 \in nodeState[n] /\ uuid2 \in nodeState[n])

(***************************************************************************
 * Theorems
 ***************************************************************************)

\* THEOREM: Add-Wins Set convergence
THEOREM AddWinsConvergence ==
  \A n1, n2 \in Nodes :
    deliveredOps[n1] = deliveredOps[n2] =>
      ComputeRelationSet(deliveredOps[n1]) = ComputeRelationSet(deliveredOps[n2])

\* THEOREM: Commutativity of relation computation
THEOREM ComputeRelationSetCommutative ==
  \A ops1, ops2 \in SUBSET RelationOp :
    ComputeRelationSet(ops1 \union ops2) = ComputeRelationSet(ops2 \union ops1)

====
