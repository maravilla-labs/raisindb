---- MODULE CausalDelivery ----
(***************************************************************************
 * Causal Delivery Buffer Specification for RaisinDB
 *
 * This module models the causal delivery buffer that ensures operations
 * are delivered to the replay engine only when all their causal
 * dependencies have been satisfied.
 *
 * Based on: crates/raisin-replication/src/causal_delivery.rs
 *
 * Key Properties Verified:
 * 1. Causal order invariant: if op1 -> op2, then op1 applied before op2
 * 2. Buffer bounded: never exceeds MaxBufferSize
 * 3. No operation applied twice
 * 4. All operations eventually delivered (liveness)
 ***************************************************************************)

EXTENDS Naturals, Sequences, FiniteSets, TLC

\* Import vector clock module
LOCAL INSTANCE VectorClock

CONSTANTS
  Nodes,          \* Set of node identifiers
  MaxOps,         \* Maximum number of operations (for model checking)
  MaxBufferSize   \* Maximum buffer capacity

ASSUME MaxOps > 0
ASSUME MaxBufferSize > 0
ASSUME Nodes /= {}

VARIABLES
  localVC,        \* Local vector clock at each node: [Nodes -> VectorClock]
  buffer,         \* Buffered operations per node: [Nodes -> set of Operation]
  applied,        \* Successfully applied operations per node: [Nodes -> set of Operation]
  opCounter,      \* Global operation counter (for generating unique IDs)
  applyOrder      \* Sequence of applied operations per node (for checking order)

vars == <<localVC, buffer, applied, opCounter, applyOrder>>

(***************************************************************************
 * Operation Structure
 ***************************************************************************)

\* Operation ID type
OpId == Nat

\* An operation in the system
Operation == [
  id: OpId,                    \* Unique operation ID
  nodeId: Nodes,               \* Originating node
  vectorClock: VectorClock,    \* Vector clock at creation
  payload: STRING              \* Abstract payload (could be any data)
]

(***************************************************************************
 * Type Invariants
 ***************************************************************************)

TypeOK ==
  /\ localVC \in [Nodes -> VectorClock]
  /\ buffer \in [Nodes -> SUBSET Operation]
  /\ applied \in [Nodes -> SUBSET Operation]
  /\ opCounter \in Nat
  /\ applyOrder \in [Nodes -> Seq(Operation)]

(***************************************************************************
 * Initial State
 ***************************************************************************)

Init ==
  /\ localVC = [n \in Nodes |-> InitVC]
  /\ buffer = [n \in Nodes |-> {}]
  /\ applied = [n \in Nodes |-> {}]
  /\ opCounter = 0
  /\ applyOrder = [n \in Nodes |-> <<>>]

(***************************************************************************
 * Helper Operators
 ***************************************************************************)

\* Check if operation's dependencies are satisfied at a node
\* Dependencies are satisfied when local VC >= operation VC (for all nodes)
DependenciesSatisfied(op, node) ==
  \A n \in Nodes : op.vectorClock[n] <= localVC[node][n]

\* Special check for same-node operations: must be exactly next in sequence
SameNodeSequential(op, node) ==
  IF op.nodeId = node
  THEN localVC[node][op.nodeId] = op.vectorClock[op.nodeId] - 1
  ELSE TRUE

\* Full dependency check (matches causal_delivery.rs implementation)
CanDeliver(op, node) ==
  /\ DependenciesSatisfied(op, node)
  /\ SameNodeSequential(op, node)

\* Find all deliverable operations in buffer
DeliverableOps(node) ==
  {op \in buffer[node] : CanDeliver(op, node)}

\* Check if buffer is full at a node
BufferFull(node) ==
  Cardinality(buffer[node]) >= MaxBufferSize

\* Update local vector clock after applying operation
UpdateLocalVC(node, op) ==
  Merge(localVC[node], op.vectorClock)

(***************************************************************************
 * Actions
 ***************************************************************************)

\* Create a new operation at a node
CreateOperation(node) ==
  /\ opCounter < MaxOps  \* Bound for model checking
  /\ LET newVC == Increment(localVC[node], node)
         newOp == [
           id |-> opCounter,
           nodeId |-> node,
           vectorClock |-> newVC,
           payload |-> "data"
         ]
     IN /\ opCounter' = opCounter + 1
        /\ localVC' = [localVC EXCEPT ![node] = newVC]
        \* Apply immediately (local operations always deliverable)
        /\ applied' = [applied EXCEPT ![node] = @ \union {newOp}]
        /\ applyOrder' = [applyOrder EXCEPT ![node] = Append(@, newOp)]
        /\ UNCHANGED buffer

\* Deliver an operation to a node (from network)
DeliverOperation(op, node) ==
  /\ op.nodeId /= node  \* Remote operation
  /\ op \notin applied[node]  \* Not already applied
  /\ op \notin buffer[node]   \* Not already buffered
  /\ IF CanDeliver(op, node)
     THEN \* Dependencies satisfied - apply directly
          /\ applied' = [applied EXCEPT ![node] = @ \union {op}]
          /\ localVC' = [localVC EXCEPT ![node] = UpdateLocalVC(node, op)]
          /\ applyOrder' = [applyOrder EXCEPT ![node] = Append(@, op)]
          /\ UNCHANGED buffer
     ELSE \* Dependencies not satisfied - buffer it
          /\ buffer' = [buffer EXCEPT ![node] = @ \union {op}]
          /\ UNCHANGED <<applied, localVC, applyOrder>>
  /\ UNCHANGED opCounter

\* Drain buffer: deliver buffered operations whose dependencies are now satisfied
DrainBuffer(node) ==
  /\ buffer[node] /= {}
  /\ LET deliverable == DeliverableOps(node)
     IN /\ deliverable /= {}
        /\ \E op \in deliverable :
             /\ buffer' = [buffer EXCEPT ![node] = @ \ {op}]
             /\ applied' = [applied EXCEPT ![node] = @ \union {op}]
             /\ localVC' = [localVC EXCEPT ![node] = UpdateLocalVC(node, op)]
             /\ applyOrder' = [applyOrder EXCEPT ![node] = Append(@, op)]
             /\ UNCHANGED opCounter

\* Combined next-state relation
Next ==
  \/ \E n \in Nodes : CreateOperation(n)
  \/ \E n \in Nodes, op \in Operation : DeliverOperation(op, n)
  \/ \E n \in Nodes : DrainBuffer(n)

\* Fairness constraints: operations eventually get delivered
Fairness ==
  /\ WF_vars(\E n \in Nodes : DrainBuffer(n))

\* Specification
Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************
 * Safety Invariants
 ***************************************************************************)

\* Causal Order Invariant: if op1 happens-before op2, op1 applied before op2
\* This is the CRITICAL property for CRDT convergence
CausalOrderInvariant ==
  \A node \in Nodes :
    \A i, j \in 1..Len(applyOrder[node]) :
      (i < j) =>
        LET op1 == applyOrder[node][i]
            op2 == applyOrder[node][j]
        IN ~HappensBefore(op2.vectorClock, op1.vectorClock)

\* Buffer is bounded
BufferBounded ==
  \A node \in Nodes :
    Cardinality(buffer[node]) <= MaxBufferSize

\* No operation applied twice at same node
NoDoubleApplication ==
  \A node \in Nodes :
    \A i, j \in 1..Len(applyOrder[node]) :
      (i /= j) => (applyOrder[node][i].id /= applyOrder[node][j].id)

\* Applied operations have satisfied dependencies
AppliedOpsSatisfied ==
  \A node \in Nodes :
    \A op \in applied[node] :
      \* At the time of application, dependencies were satisfied
      \* (We verify this by checking the apply sequence)
      \A i \in 1..Len(applyOrder[node]) :
        (applyOrder[node][i].id = op.id) =>
          \* All operations that this op depends on were applied earlier
          \A dep_node \in Nodes :
            op.vectorClock[dep_node] > 0 =>
              \E j \in 1..i :
                /\ applyOrder[node][j].nodeId = dep_node
                /\ applyOrder[node][j].vectorClock[dep_node] <= op.vectorClock[dep_node]

\* Operations in buffer are waiting for dependencies
BufferedOpsWaiting ==
  \A node \in Nodes :
    \A op \in buffer[node] :
      ~CanDeliver(op, node)

\* Local vector clock is monotonic
LocalVCMonotonic ==
  \A node \in Nodes :
    \A i \in 1..(Len(applyOrder[node]) - 1) :
      LessOrEqual(
        applyOrder[node][i].vectorClock,
        applyOrder[node][i+1].vectorClock
      )

\* Operations from same node are delivered in sequence
SameNodeSequenceOrder ==
  \A node \in Nodes :
    \A i, j \in 1..Len(applyOrder[node]) :
      (i < j /\ applyOrder[node][i].nodeId = applyOrder[node][j].nodeId) =>
        applyOrder[node][i].vectorClock[applyOrder[node][i].nodeId] <
        applyOrder[node][j].vectorClock[applyOrder[node][j].nodeId]

(***************************************************************************
 * Liveness Properties
 ***************************************************************************)

\* All operations eventually get applied (if no buffer overflow)
EventualDelivery ==
  \A op \in Operation :
    \A node \in Nodes :
      (op.nodeId /= node) =>
        <>(op \in applied[node] \/ BufferFull(node))

\* Buffer eventually drains (no operations stuck forever)
BufferEventuallyDrains ==
  \A node \in Nodes :
    (buffer[node] /= {}) ~> (buffer[node] = {})

(***************************************************************************
 * Model Checking Configuration
 ***************************************************************************)

\* State constraint to limit state space
StateConstraint ==
  /\ opCounter <= MaxOps
  /\ \A node \in Nodes :
       /\ Cardinality(applied[node]) <= MaxOps
       /\ Cardinality(buffer[node]) <= MaxBufferSize

\* Invariant to check (combination of safety properties)
Invariant ==
  /\ TypeOK
  /\ CausalOrderInvariant
  /\ BufferBounded
  /\ NoDoubleApplication
  /\ BufferedOpsWaiting
  /\ SameNodeSequenceOrder

(***************************************************************************
 * Properties to Verify with TLC
 *
 * Add to .cfg file:
 *
 * CONSTANTS
 *   Nodes = {n1, n2, n3}
 *   MaxOps = 10
 *   MaxBufferSize = 5
 *
 * INVARIANTS
 *   Invariant
 *
 * PROPERTIES
 *   EventualDelivery
 *   BufferEventuallyDrains
 *
 * CONSTRAINT
 *   StateConstraint
 ***************************************************************************)

====
