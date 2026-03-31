---- MODULE DeleteWins ----
(***************************************************************************
 * Delete-Wins CRDT Specification for RaisinDB
 *
 * This module formalizes the Delete-Wins semantics used for:
 * - Node deletion (DeleteNodeSnapshot operations)
 * - Ensuring deleted nodes stay deleted (no resurrection)
 *
 * Implementation Mapping:
 * - Rust: crates/raisin-rocksdb/src/replication/application.rs
 *   - Lines 1501-1559: apply_delete_node_snapshot
 *   - Line 1503: "Delete-Wins semantics - deletions always take precedence"
 *   - Lines 1542-1552: Writes tombstones with revision HLC
 * - Rust: crates/raisin-replication/src/operation.rs
 *   - Lines 217-230: UpsertNodeSnapshot/DeleteNodeSnapshot operations
 *
 * Key Semantic: Delete-Wins
 * - If upsert and delete are concurrent, the node is DELETED
 * - Once deleted, a node cannot be resurrected by earlier upserts
 * - Delete operations write tombstones that persist
 *
 * Properties Verified:
 * 1. Delete-Wins: Concurrent upsert/delete -> node deleted
 * 2. No Resurrection: Once deleted with later timestamp, stays deleted
 * 3. Convergence: Same operations -> same final state
 * 4. Tombstone Persistence: Deletes create persistent markers
 ***************************************************************************)

EXTENDS Naturals, TLC, FiniteSets
INSTANCE VectorClock

CONSTANTS
  Nodes,          \* Set of cluster nodes
  NodeIds,        \* Set of node IDs that can be created/deleted
  MaxTimestamp,   \* Maximum HLC timestamp
  MaxCounter,     \* Maximum HLC counter
  MaxOps          \* Maximum operations for model checking

(***************************************************************************
 * Hybrid Logical Clock (HLC)
 ***************************************************************************)

HLC == [
  timestamp_ms: 0..MaxTimestamp,
  counter: 0..MaxCounter
]

InitHLC == [timestamp_ms |-> 0, counter |-> 0]

\* HLC ordering (same as LWW.tla)
HLCBefore(hlc1, hlc2) ==
  \/ hlc1.timestamp_ms < hlc2.timestamp_ms
  \/ (hlc1.timestamp_ms = hlc2.timestamp_ms /\ hlc1.counter < hlc2.counter)

HLCAfter(hlc1, hlc2) ==
  HLCBefore(hlc2, hlc1)

HLCEqual(hlc1, hlc2) ==
  /\ hlc1.timestamp_ms = hlc2.timestamp_ms
  /\ hlc1.counter = hlc2.counter

(***************************************************************************
 * Node Operations
 *
 * Two types of operations:
 * 1. UPSERT: Creates or updates a node with HLC revision
 * 2. DELETE: Marks node as deleted with HLC revision (tombstone)
 ***************************************************************************)

UpsertOp == [
  opType: {"UPSERT"},
  nodeId: NodeIds,
  value: STRING,
  revision: HLC,
  vc: VectorClock,
  sourceNode: Nodes
]

DeleteOp == [
  opType: {"DELETE"},
  nodeId: NodeIds,
  revision: HLC,
  vc: VectorClock,
  sourceNode: Nodes
]

NodeOp == UpsertOp \union DeleteOp

(***************************************************************************
 * Delete-Wins Semantics
 *
 * A node is DELETED if and only if:
 * - There exists a DELETE operation for the node, AND
 * - For all UPSERT operations for the node, either:
 *   a) The upsert's revision is before the delete's revision, OR
 *   b) The upsert is concurrent with delete (Delete-Wins)
 *
 * In other words: Delete wins if it's concurrent or happens-after any upsert
 ***************************************************************************)

\* Check if a node is deleted given the set of operations
NodeDeleted(nodeId, ops) ==
  \E delOp \in ops :
    /\ delOp.opType = "DELETE"
    /\ delOp.nodeId = nodeId
    /\ \A upsertOp \in ops :
        (upsertOp.nodeId = nodeId /\ upsertOp.opType = "UPSERT") =>
          \/ HLCBefore(upsertOp.revision, delOp.revision)
          \/ HLCEqual(upsertOp.revision, delOp.revision)  \* Concurrent -> Delete wins
          \* Note: Using revision HLC for Delete-Wins, not vector clock

\* Alternative: Node exists (not deleted) if latest revision is upsert
NodeExists(nodeId, ops) ==
  /\ \E upsertOp \in ops :
       /\ upsertOp.opType = "UPSERT"
       /\ upsertOp.nodeId = nodeId
  /\ ~NodeDeleted(nodeId, ops)

\* Get latest operation for a node (by revision HLC)
LatestOperation(nodeId, ops) ==
  LET nodeOps == {op \in ops : op.nodeId = nodeId}
  IN IF nodeOps = {} THEN InitHLC
     ELSE CHOOSE op \in nodeOps :
       \A other \in nodeOps :
         ~HLCBefore(op.revision, other.revision)

(***************************************************************************
 * State Machine for Testing Delete-Wins
 ***************************************************************************)

VARIABLES
  operations,     \* Global history of all operations
  nodeState,      \* Per-node view: [Nodes -> [NodeIds -> {"EXISTS", "DELETED", "NONE"}]]
  nodeVC,         \* Vector clock per node: [Nodes -> VectorClock]
  deliveredOps    \* Operations delivered to each node: [Nodes -> SUBSET operations]

dw_vars == <<operations, nodeState, nodeVC, deliveredOps>>

(***************************************************************************
 * Type Invariants
 ***************************************************************************)

TypeOK ==
  /\ operations \subseteq NodeOp
  /\ nodeState \in [Nodes -> [NodeIds -> {"EXISTS", "DELETED", "NONE"}]]
  /\ nodeVC \in [Nodes -> VectorClock]
  /\ deliveredOps \in [Nodes -> SUBSET operations]

(***************************************************************************
 * Initial State
 ***************************************************************************)

Init ==
  /\ operations = {}
  /\ nodeState = [n \in Nodes |-> [nid \in NodeIds |-> "NONE"]]
  /\ nodeVC = [n \in Nodes |-> InitVC]
  /\ deliveredOps = [n \in Nodes |-> {}]

(***************************************************************************
 * Helper Functions
 ***************************************************************************)

\* Compute node state from delivered operations
ComputeNodeState(nodeId, ops) ==
  IF NodeDeleted(nodeId, ops) THEN "DELETED"
  ELSE IF NodeExists(nodeId, ops) THEN "EXISTS"
  ELSE "NONE"

(***************************************************************************
 * Actions
 ***************************************************************************)

\* Upsert a node (create or update)
UpsertNode(node, nodeId, value, revision) ==
  /\ Cardinality(operations) < MaxOps
  /\ LET vc == Increment(nodeVC[node], node)
         op == [
           opType |-> "UPSERT",
           nodeId |-> nodeId,
           value |-> value,
           revision |-> revision,
           vc |-> vc,
           sourceNode |-> node
         ]
     IN /\ operations' = operations \union {op}
        /\ nodeVC' = [nodeVC EXCEPT ![node] = vc]
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node][nodeId] =
                         ComputeNodeState(nodeId, deliveredOps'[node])]

\* Delete a node
DeleteNode(node, nodeId, revision) ==
  /\ Cardinality(operations) < MaxOps
  /\ LET vc == Increment(nodeVC[node], node)
         op == [
           opType |-> "DELETE",
           nodeId |-> nodeId,
           revision |-> revision,
           vc |-> vc,
           sourceNode |-> node
         ]
     IN /\ operations' = operations \union {op}
        /\ nodeVC' = [nodeVC EXCEPT ![node] = vc]
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node][nodeId] =
                         ComputeNodeState(nodeId, deliveredOps'[node])]

\* Deliver an operation to a node
DeliverOp(node, op) ==
  /\ op \in operations
  /\ op \notin deliveredOps[node]
  /\ LET mergedVC == Merge(nodeVC[node], op.vc)
         newVC == Increment(mergedVC, node)
     IN /\ nodeVC' = [nodeVC EXCEPT ![node] = newVC]
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node][op.nodeId] =
                         ComputeNodeState(op.nodeId, deliveredOps'[node])]
        /\ UNCHANGED operations

(***************************************************************************
 * Next State Relation
 ***************************************************************************)

Next ==
  \/ \E node \in Nodes, nid \in NodeIds, val \in {"v1", "v2"},
         ts \in 1..MaxTimestamp, cnt \in 0..MaxCounter :
       UpsertNode(node, nid, val, [timestamp_ms |-> ts, counter |-> cnt])
  \/ \E node \in Nodes, nid \in NodeIds,
         ts \in 1..MaxTimestamp, cnt \in 0..MaxCounter :
       DeleteNode(node, nid, [timestamp_ms |-> ts, counter |-> cnt])
  \/ \E node \in Nodes, op \in operations :
       DeliverOp(node, op)

Spec == Init /\ [][Next]_dw_vars

(***************************************************************************
 * Key Properties
 ***************************************************************************)

\* PROPERTY 1: Delete-Wins Semantics
\* If upsert and delete are concurrent (by revision), node is deleted
DeleteWinsProperty ==
  \A upsertOp \in operations, delOp \in operations :
    /\ upsertOp.opType = "UPSERT"
    /\ delOp.opType = "DELETE"
    /\ upsertOp.nodeId = delOp.nodeId
    /\ HLCEqual(upsertOp.revision, delOp.revision)  \* Concurrent by revision
    =>
      \A n \in Nodes :
        (upsertOp \in deliveredOps[n] /\ delOp \in deliveredOps[n]) =>
          nodeState[n][upsertOp.nodeId] = "DELETED"

\* PROPERTY 2: No Resurrection
\* Once a node is deleted with a certain revision, any upsert with earlier
\* or equal revision cannot bring it back
NoResurrection ==
  \A n \in Nodes, nid \in NodeIds :
    nodeState[n][nid] = "DELETED" =>
      \E delOp \in deliveredOps[n] :
        /\ delOp.opType = "DELETE"
        /\ delOp.nodeId = nid
        /\ \A upsertOp \in deliveredOps[n] :
             (upsertOp.opType = "UPSERT" /\ upsertOp.nodeId = nid) =>
               ~HLCAfter(upsertOp.revision, delOp.revision)

\* PROPERTY 3: Convergence
\* Nodes with same delivered operations have same state
Convergence ==
  \A n1, n2 \in Nodes :
    deliveredOps[n1] = deliveredOps[n2] =>
      nodeState[n1] = nodeState[n2]

\* PROPERTY 4: Eventual Consistency
\* Once all nodes receive all operations, they all agree
EventualConsistency ==
  (\A n \in Nodes : deliveredOps[n] = operations) =>
    \A n1, n2 \in Nodes : nodeState[n1] = nodeState[n2]

\* PROPERTY 5: Revision Monotonicity
\* If a node transitions from exists to deleted, delete revision > any upsert revision
RevisionMonotonicity ==
  \A n \in Nodes, nid \in NodeIds :
    nodeState[n][nid] = "DELETED" =>
      LET delOps == {op \in deliveredOps[n] : op.opType = "DELETE" /\ op.nodeId = nid}
          upsertOps == {op \in deliveredOps[n] : op.opType = "UPSERT" /\ op.nodeId = nid}
      IN delOps # {} =>
        \E delOp \in delOps :
          \A upsertOp \in upsertOps :
            HLCBefore(upsertOp.revision, delOp.revision) \/ HLCEqual(upsertOp.revision, delOp.revision)

\* PROPERTY 6: Delete Idempotency
\* Deleting multiple times with same revision has same effect as once
DeleteIdempotency ==
  \A ops1, ops2 \in SUBSET NodeOp :
    \A nid \in NodeIds :
      ComputeNodeState(nid, ops1) = ComputeNodeState(nid, ops2 \union ops2)

(***************************************************************************
 * Invariants for Testing
 ***************************************************************************)

\* Delivered operations are subset of global operations
DeliveredOpsValid ==
  \A n \in Nodes : deliveredOps[n] \subseteq operations

\* Vector clocks are monotonic
VectorClockMonotonic ==
  \A n \in Nodes : nodeVC[n][n] >= 0

\* Node state is consistent with delivered operations
NodeStateConsistent ==
  \A n \in Nodes, nid \in NodeIds :
    nodeState[n][nid] = ComputeNodeState(nid, deliveredOps[n])

\* If node is deleted, there must be a delete operation
DeletedHasDeleteOp ==
  \A n \in Nodes, nid \in NodeIds :
    nodeState[n][nid] = "DELETED" =>
      \E op \in deliveredOps[n] :
        op.opType = "DELETE" /\ op.nodeId = nid

(***************************************************************************
 * Example Scenarios
 ***************************************************************************)

\* Scenario 1: Concurrent upsert and delete
\* Node1 upserts with revision (100, 0)
\* Node2 deletes with revision (100, 0)
\* Result: Node should be DELETED (Delete-Wins)
ConcurrentUpsertDeleteScenario ==
  \E upsertOp \in operations, delOp \in operations :
    /\ upsertOp.opType = "UPSERT"
    /\ delOp.opType = "DELETE"
    /\ upsertOp.nodeId = delOp.nodeId
    /\ HLCEqual(upsertOp.revision, delOp.revision)
    =>
      \A n \in Nodes :
        (upsertOp \in deliveredOps[n] /\ delOp \in deliveredOps[n]) =>
          nodeState[n][upsertOp.nodeId] = "DELETED"

\* Scenario 2: Upsert after delete (resurrection attempt)
\* Delete with revision (100, 0)
\* Upsert with revision (99, 0) arrives later
\* Result: Node stays DELETED (earlier revision cannot resurrect)
NoResurrectionScenario ==
  \E delOp \in operations, upsertOp \in operations :
    /\ delOp.opType = "DELETE"
    /\ upsertOp.opType = "UPSERT"
    /\ delOp.nodeId = upsertOp.nodeId
    /\ HLCBefore(upsertOp.revision, delOp.revision)
    =>
      \A n \in Nodes :
        (delOp \in deliveredOps[n] /\ upsertOp \in deliveredOps[n]) =>
          nodeState[n][delOp.nodeId] = "DELETED"

\* Scenario 3: Delete with older revision than upsert
\* Upsert with revision (100, 0)
\* Delete with revision (99, 0)
\* Result: Node EXISTS (upsert has later revision)
OlderDeleteScenario ==
  \E upsertOp \in operations, delOp \in operations :
    /\ upsertOp.opType = "UPSERT"
    /\ delOp.opType = "DELETE"
    /\ upsertOp.nodeId = delOp.nodeId
    /\ HLCAfter(upsertOp.revision, delOp.revision)
    =>
      \A n \in Nodes :
        (upsertOp \in deliveredOps[n] /\ delOp \in deliveredOps[n]) =>
          nodeState[n][upsertOp.nodeId] = "EXISTS"

(***************************************************************************
 * Theorems
 ***************************************************************************)

\* THEOREM: Delete-Wins is deterministic
THEOREM DeleteWinsDeterministic ==
  \A ops1, ops2 \in SUBSET NodeOp :
    \A nid \in NodeIds :
      ops1 = ops2 => ComputeNodeState(nid, ops1) = ComputeNodeState(nid, ops2)

\* THEOREM: Delete-Wins computation is commutative
THEOREM DeleteWinsCommutative ==
  \A ops1, ops2 \in SUBSET NodeOp :
    \A nid \in NodeIds :
      ComputeNodeState(nid, ops1 \union ops2) = ComputeNodeState(nid, ops2 \union ops1)

====
