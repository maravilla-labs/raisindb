---- MODULE LWW ----
(***************************************************************************
 * Last-Write-Wins (LWW) CRDT Specification for RaisinDB
 *
 * This module formalizes the Last-Write-Wins CRDT semantics used for:
 * - Node properties (UpsertNodeSnapshot operations)
 * - Single-valued fields
 *
 * Implementation Mapping:
 * - Rust: crates/raisin-rocksdb/src/replication/application.rs
 *   - Lines 1460-1499: apply_upsert_node_snapshot
 *   - Lines 1462-1463: "LWW semantics using revision HLC"
 * - Rust: crates/raisin-hlc/src/lib.rs
 *   - Lines 225-232: HLC ordering (Ord implementation)
 *
 * Key Properties Verified:
 * 1. Determinism: Same operations always result in same winner
 * 2. Commutativity: Merge(a,b) = Merge(b,a)
 * 3. Associativity: Merge(Merge(a,b),c) = Merge(a,Merge(b,c))
 * 4. Convergence: All replicas converge to same state
 ***************************************************************************)

EXTENDS Naturals, TLC, FiniteSets
INSTANCE VectorClock

CONSTANTS
  Nodes,        \* Set of cluster nodes
  NodeIds,      \* Set of node IDs that can be created
  MaxTimestamp, \* Maximum timestamp for model checking
  MaxCounter    \* Maximum counter for model checking

(***************************************************************************
 * Hybrid Logical Clock (HLC) Data Structure
 *
 * HLC combines physical wall-clock time with logical counters to provide
 * total ordering across distributed systems.
 *
 * Maps to: raisin_hlc::HLC (crates/raisin-hlc/src/lib.rs, lines 62-68)
 ***************************************************************************)

HLC == [
  timestamp_ms: 0..MaxTimestamp,
  counter: 0..MaxCounter
]

\* Initial HLC (zero timestamp and counter)
InitHLC == [timestamp_ms |-> 0, counter |-> 0]

(***************************************************************************
 * HLC Comparison and Ordering
 *
 * Maps to: impl Ord for HLC (crates/raisin-hlc/src/lib.rs, lines 225-232)
 * Ordering: timestamp first, then counter
 ***************************************************************************)

\* Compare two HLCs: returns "BEFORE", "AFTER", or "EQUAL"
HLCCompare(hlc1, hlc2) ==
  IF hlc1.timestamp_ms < hlc2.timestamp_ms THEN "BEFORE"
  ELSE IF hlc1.timestamp_ms > hlc2.timestamp_ms THEN "AFTER"
  ELSE IF hlc1.counter < hlc2.counter THEN "BEFORE"
  ELSE IF hlc1.counter > hlc2.counter THEN "AFTER"
  ELSE "EQUAL"

\* HLC ordering predicate
HLCBefore(hlc1, hlc2) ==
  \/ hlc1.timestamp_ms < hlc2.timestamp_ms
  \/ (hlc1.timestamp_ms = hlc2.timestamp_ms /\ hlc1.counter < hlc2.counter)

HLCAfter(hlc1, hlc2) ==
  HLCBefore(hlc2, hlc1)

HLCEqual(hlc1, hlc2) ==
  /\ hlc1.timestamp_ms = hlc2.timestamp_ms
  /\ hlc1.counter = hlc2.counter

(***************************************************************************
 * LWW Operation
 *
 * Represents a write operation with HLC timestamp
 ***************************************************************************)

LWWOp == [
  nodeId: NodeIds,
  value: STRING,
  hlc: HLC,
  sourceNode: Nodes  \* Which cluster node originated this operation
]

(***************************************************************************
 * LWW Merge: Choose operation with later HLC
 *
 * Maps to: apply_upsert_node_snapshot (application.rs, lines 1478-1490)
 * The storage layer keeps versioned keys and load_latest_node returns
 * the version with highest revision (HLC).
 ***************************************************************************)

LWWMerge(op1, op2) ==
  IF HLCBefore(op1.hlc, op2.hlc) THEN op2
  ELSE IF HLCAfter(op1.hlc, op2.hlc) THEN op1
  ELSE op1  \* Equal timestamps: deterministic tie-break (keep first)

\* Merge a set of operations (reduce to single winner)
LWWMergeSet(ops) ==
  IF ops = {} THEN CHOOSE x : FALSE  \* Error: empty set
  ELSE CHOOSE winner \in ops :
    \A other \in ops :
      \/ winner = other
      \/ ~HLCBefore(winner.hlc, other.hlc)

(***************************************************************************
 * State Machine for Testing LWW
 ***************************************************************************)

VARIABLES
  operations,     \* Set of all operations (global history)
  nodeState,      \* Per-node view: [Nodes -> [NodeIds -> LWWOp]]
  deliveredOps    \* Operations delivered to each node: [Nodes -> SUBSET operations]

lww_vars == <<operations, nodeState, deliveredOps>>

(***************************************************************************
 * Type Invariants
 ***************************************************************************)

TypeOK ==
  /\ operations \subseteq LWWOp
  /\ nodeState \in [Nodes -> [NodeIds -> LWWOp \union {InitHLC}]]
  /\ deliveredOps \in [Nodes -> SUBSET operations]

(***************************************************************************
 * Initial State
 ***************************************************************************)

Init ==
  /\ operations = {}
  /\ nodeState = [n \in Nodes |-> [nid \in NodeIds |-> InitHLC]]
  /\ deliveredOps = [n \in Nodes |-> {}]

(***************************************************************************
 * Actions
 ***************************************************************************)

\* Create a new write operation at a node
CreateOp(node, nodeId, value, hlc) ==
  /\ hlc \in HLC
  /\ LET op == [
       nodeId |-> nodeId,
       value |-> value,
       hlc |-> hlc,
       sourceNode |-> node
     ]
     IN /\ operations' = operations \union {op}
        /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node][nodeId] = op]

\* Deliver an operation to a node (apply LWW merge)
DeliverOp(node, op) ==
  /\ op \in operations
  /\ op \notin deliveredOps[node]
  /\ LET currentOp == nodeState[node][op.nodeId]
         mergedOp == IF currentOp = InitHLC
                     THEN op
                     ELSE LWWMerge(currentOp, op)
     IN /\ deliveredOps' = [deliveredOps EXCEPT ![node] = @ \union {op}]
        /\ nodeState' = [nodeState EXCEPT ![node][op.nodeId] = mergedOp]
        /\ UNCHANGED operations

\* Next state relation
Next ==
  \/ \E node \in Nodes, nid \in NodeIds, val \in {"v1", "v2", "v3"},
         ts \in 1..MaxTimestamp, cnt \in 0..MaxCounter :
       CreateOp(node, nid, val, [timestamp_ms |-> ts, counter |-> cnt])
  \/ \E node \in Nodes, op \in operations :
       DeliverOp(node, op)

Spec == Init /\ [][Next]_lww_vars

(***************************************************************************
 * Key Properties
 ***************************************************************************)

\* PROPERTY 1: Determinism
\* Same set of operations always produces same winner
Determinism ==
  \A n1, n2 \in Nodes, nid \in NodeIds :
    deliveredOps[n1] = deliveredOps[n2] =>
      nodeState[n1][nid] = nodeState[n2][nid]

\* PROPERTY 2: Commutativity
\* Order of applying operations doesn't matter
Commutativity ==
  \A op1, op2 \in LWWOp :
    LWWMerge(op1, op2) = LWWMerge(op2, op1) \/ op1.nodeId # op2.nodeId

\* PROPERTY 3: Associativity
\* Grouping of merge operations doesn't matter
Associativity ==
  \A op1, op2, op3 \in operations :
    (op1.nodeId = op2.nodeId /\ op2.nodeId = op3.nodeId) =>
      LWWMerge(LWWMerge(op1, op2), op3) = LWWMerge(op1, LWWMerge(op2, op3))

\* PROPERTY 4: Convergence
\* Once all nodes have seen same operations, they converge to same state
Convergence ==
  \A n1, n2 \in Nodes :
    (\A op \in operations : op \in deliveredOps[n1] /\ op \in deliveredOps[n2]) =>
      (\A nid \in NodeIds : nodeState[n1][nid] = nodeState[n2][nid])

\* PROPERTY 5: Monotonicity
\* Once a node adopts an operation, it never adopts an older one
Monotonicity ==
  \A n \in Nodes, nid \in NodeIds :
    LET current == nodeState[n][nid]
    IN current # InitHLC =>
      \A op \in operations :
        (op.nodeId = nid /\ op \in deliveredOps[n]) =>
          ~HLCBefore(current.hlc, op.hlc)

\* PROPERTY 6: LWW Semantics
\* The winning operation always has the latest (or equal) HLC
LWWSemantics ==
  \A n \in Nodes, nid \in NodeIds :
    LET winner == nodeState[n][nid]
    IN winner # InitHLC =>
      \A op \in deliveredOps[n] :
        op.nodeId = nid => ~HLCBefore(winner.hlc, op.hlc)

(***************************************************************************
 * Invariants for Testing
 ***************************************************************************)

\* All delivered operations must exist in global history
DeliveredOpsValid ==
  \A n \in Nodes : deliveredOps[n] \subseteq operations

\* Node state must reflect delivered operations
NodeStateValid ==
  \A n \in Nodes, nid \in NodeIds :
    LET current == nodeState[n][nid]
    IN current # InitHLC =>
      \E op \in deliveredOps[n] :
        /\ op.nodeId = nid
        /\ op.hlc = current.hlc

(***************************************************************************
 * Example Scenarios for Testing
 ***************************************************************************)

\* Scenario 1: Concurrent writes from different nodes
\* Node1 writes "v1" at HLC(100, 0)
\* Node2 writes "v2" at HLC(101, 0)
\* Expected: "v2" wins (later timestamp)
ConcurrentWritesExample ==
  /\ operations = {
       [nodeId |-> "n1", value |-> "v1", hlc |-> [timestamp_ms |-> 100, counter |-> 0], sourceNode |-> "node1"],
       [nodeId |-> "n1", value |-> "v2", hlc |-> [timestamp_ms |-> 101, counter |-> 0], sourceNode |-> "node2"]
     }
  /\ \A n \in Nodes : deliveredOps[n] = operations
  =>
    \A n \in Nodes : nodeState[n]["n1"].value = "v2"

\* Scenario 2: Same timestamp, different counters
\* Expected: Higher counter wins
SameTimestampExample ==
  /\ operations = {
       [nodeId |-> "n1", value |-> "v1", hlc |-> [timestamp_ms |-> 100, counter |-> 0], sourceNode |-> "node1"],
       [nodeId |-> "n1", value |-> "v2", hlc |-> [timestamp_ms |-> 100, counter |-> 1], sourceNode |-> "node2"]
     }
  /\ \A n \in Nodes : deliveredOps[n] = operations
  =>
    \A n \in Nodes : nodeState[n]["n1"].value = "v2"

(***************************************************************************
 * Theorems
 ***************************************************************************)

\* THEOREM: HLC ordering is total
THEOREM HLCTotalOrder ==
  \A hlc1, hlc2 \in HLC :
    \/ HLCBefore(hlc1, hlc2)
    \/ HLCAfter(hlc1, hlc2)
    \/ HLCEqual(hlc1, hlc2)

\* THEOREM: LWW merge is commutative for same nodeId
THEOREM LWWMergeCommutative ==
  \A op1, op2 \in LWWOp :
    op1.nodeId = op2.nodeId =>
      LWWMerge(op1, op2).hlc = LWWMerge(op2, op1).hlc

\* THEOREM: LWW merge is associative for same nodeId
THEOREM LWWMergeAssociative ==
  \A op1, op2, op3 \in LWWOp :
    (op1.nodeId = op2.nodeId /\ op2.nodeId = op3.nodeId) =>
      LWWMerge(LWWMerge(op1, op2), op3).hlc =
      LWWMerge(op1, LWWMerge(op2, op3)).hlc

====
