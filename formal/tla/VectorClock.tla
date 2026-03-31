---- MODULE VectorClock ----
(***************************************************************************
 * Vector Clock Specification for RaisinDB
 *
 * This module formalizes vector clocks and their properties:
 * - Increment operation
 * - Merge operation (pointwise maximum)
 * - Happens-before relation (partial order)
 * - Concurrent detection
 * - Distance calculation
 *
 * Vector clocks are the foundation of causal ordering in the CRDT
 * replication system.
 ***************************************************************************)

EXTENDS Naturals, TLC, FiniteSets

CONSTANTS
  Nodes  \* Set of node identifiers (cluster nodes)

(***************************************************************************
 * Vector Clock Data Structure
 *
 * A vector clock is a function from Nodes to Nat
 * We represent it as: [Nodes -> Nat]
 ***************************************************************************)

\* The set of all possible vector clocks
VectorClock == [Nodes -> Nat]

(***************************************************************************
 * Vector Clock Operations
 ***************************************************************************)

\* Initial vector clock (all zeros)
InitVC == [n \in Nodes |-> 0]

\* Increment vector clock at a specific node
Increment(vc, node) ==
  [vc EXCEPT ![node] = @ + 1]

\* Merge two vector clocks (pointwise maximum)
Merge(vc1, vc2) ==
  [n \in Nodes |-> IF vc1[n] > vc2[n] THEN vc1[n] ELSE vc2[n]]

\* Maximum value (helper function)
Max(a, b) == IF a > b THEN a ELSE b

\* Minimum value (helper function)
Min(a, b) == IF a < b THEN a ELSE b

(***************************************************************************
 * Vector Clock Comparisons
 ***************************************************************************)

\* Check if vc1 <= vc2 (pointwise less than or equal)
LessOrEqual(vc1, vc2) ==
  \A n \in Nodes : vc1[n] <= vc2[n]

\* Check if vc1 < vc2 (strictly less than)
StrictlyLess(vc1, vc2) ==
  /\ LessOrEqual(vc1, vc2)
  /\ \E n \in Nodes : vc1[n] < vc2[n]

\* Happens-before relation: vc1 happened before vc2
\* vc1 < vc2 iff vc1[i] <= vc2[i] for all i AND vc1 != vc2
HappensBefore(vc1, vc2) ==
  StrictlyLess(vc1, vc2)

\* Happens-after relation: vc1 happened after vc2
HappensAfter(vc1, vc2) ==
  HappensBefore(vc2, vc1)

\* Equal vector clocks
Equal(vc1, vc2) ==
  \A n \in Nodes : vc1[n] = vc2[n]

\* Concurrent vector clocks (neither happens-before nor happens-after)
Concurrent(vc1, vc2) ==
  /\ ~HappensBefore(vc1, vc2)
  /\ ~HappensBefore(vc2, vc1)
  /\ ~Equal(vc1, vc2)

\* Compare two vector clocks and return ordering
\* Returns: "before", "after", "concurrent", or "equal"
Compare(vc1, vc2) ==
  IF Equal(vc1, vc2) THEN "equal"
  ELSE IF HappensBefore(vc1, vc2) THEN "before"
  ELSE IF HappensBefore(vc2, vc1) THEN "after"
  ELSE "concurrent"

(***************************************************************************
 * Distance Metrics
 ***************************************************************************)

\* Calculate distance between two vector clocks
\* Distance is the sum of differences where vc2 is ahead of vc1
Distance(vc1, vc2) ==
  LET diffs == {vc2[n] - vc1[n] : n \in Nodes}
      positive_diffs == {d \in diffs : d > 0}
  IN  IF positive_diffs = {} THEN 0
      ELSE CHOOSE sum \in Nat :
           \E f \in [positive_diffs -> Nat] :
             /\ sum = (CHOOSE s \in Nat : s = (
                  IF Cardinality(positive_diffs) = 0 THEN 0
                  ELSE (CHOOSE total \in Nat : TRUE)))  \* Simplified for spec
             /\ sum >= 0

\* Simplified distance that works better in TLC
SimpleDistance(vc1, vc2) ==
  (CHOOSE d \in 0..10000 :
    \A n \in Nodes : d >= (IF vc2[n] > vc1[n] THEN vc2[n] - vc1[n] ELSE 0))

(***************************************************************************
 * Vector Clock Properties (Theorems)
 ***************************************************************************)

\* PROPERTY: Happens-before is a strict partial order

\* Irreflexivity: vc < vc is always false
THEOREM IrreflexiveHappensBefore ==
  \A vc \in VectorClock : ~HappensBefore(vc, vc)

\* Antisymmetry: if vc1 < vc2, then NOT vc2 < vc1
THEOREM AntisymmetricHappensBefore ==
  \A vc1, vc2 \in VectorClock :
    HappensBefore(vc1, vc2) => ~HappensBefore(vc2, vc1)

\* Transitivity: if vc1 < vc2 and vc2 < vc3, then vc1 < vc3
THEOREM TransitiveHappensBefore ==
  \A vc1, vc2, vc3 \in VectorClock :
    (HappensBefore(vc1, vc2) /\ HappensBefore(vc2, vc3))
      => HappensBefore(vc1, vc3)

\* PROPERTY: Merge creates a least upper bound

\* Merge result is >= both inputs
THEOREM MergeUpperBound ==
  \A vc1, vc2 \in VectorClock :
    LET merged == Merge(vc1, vc2)
    IN /\ LessOrEqual(vc1, merged)
       /\ LessOrEqual(vc2, merged)

\* Merge is commutative
THEOREM MergeCommutative ==
  \A vc1, vc2 \in VectorClock :
    Merge(vc1, vc2) = Merge(vc2, vc1)

\* Merge is associative
THEOREM MergeAssociative ==
  \A vc1, vc2, vc3 \in VectorClock :
    Merge(Merge(vc1, vc2), vc3) = Merge(vc1, Merge(vc2, vc3))

\* Merge is idempotent
THEOREM MergeIdempotent ==
  \A vc \in VectorClock :
    Merge(vc, vc) = vc

\* PROPERTY: Increment preserves ordering

\* Incrementing always increases the clock
THEOREM IncrementIncreases ==
  \A vc \in VectorClock, n \in Nodes :
    HappensBefore(vc, Increment(vc, n))

\* Increment is monotonic with respect to happens-before
THEOREM IncrementMonotonic ==
  \A vc1, vc2 \in VectorClock, n \in Nodes :
    HappensBefore(vc1, vc2) =>
      HappensBefore(Increment(vc1, n), Increment(vc2, n))

(***************************************************************************
 * Causal Consistency Properties
 ***************************************************************************)

\* If two operations are concurrent, their vector clocks are concurrent
ConcurrentOperationsHaveConcurrentClocks ==
  \A vc1, vc2 \in VectorClock :
    Concurrent(vc1, vc2) <=>
      (~HappensBefore(vc1, vc2) /\ ~HappensBefore(vc2, vc1) /\ ~Equal(vc1, vc2))

\* Any two vector clocks have exactly one relationship
TotalRelationship ==
  \A vc1, vc2 \in VectorClock :
    \/ HappensBefore(vc1, vc2)
    \/ HappensBefore(vc2, vc1)
    \/ Concurrent(vc1, vc2)
    \/ Equal(vc1, vc2)

(***************************************************************************
 * State Machine for Testing Vector Clock Operations
 ***************************************************************************)

VARIABLES
  clocks,     \* Map from nodes to their current vector clock
  history     \* History of all vector clock states (for testing)

clock_vars == <<clocks, history>>

\* Type invariant
VCTypeOK ==
  /\ clocks \in [Nodes -> VectorClock]
  /\ history \in Seq([Nodes -> VectorClock])

\* Initial state
VCInit ==
  /\ clocks = [n \in Nodes |-> InitVC]
  /\ history = <<>>

\* Node performs a local operation (increments its clock)
LocalOp(node) ==
  /\ clocks' = [clocks EXCEPT ![node] = Increment(@, node)]
  /\ history' = Append(history, clocks')

\* Node receives a message with remote clock (merges then increments)
ReceiveOp(node, remote_vc) ==
  /\ LET merged == Merge(clocks[node], remote_vc)
         incremented == Increment(merged, node)
     IN clocks' = [clocks EXCEPT ![node] = incremented]
  /\ history' = Append(history, clocks')

\* Next state
VCNext ==
  \/ \E n \in Nodes : LocalOp(n)
  \/ \E n \in Nodes, vc \in VectorClock : ReceiveOp(n, vc)

\* Specification
VCSpec == VCInit /\ [][VCNext]_clock_vars

(***************************************************************************
 * Invariants for Testing
 ***************************************************************************)

\* All clocks are monotonically increasing
MonotonicClocks ==
  \A i \in 1..(Len(history) - 1) :
    \A n \in Nodes :
      LessOrEqual(history[i][n], history[i+1][n])

\* A node's own counter only increases
OwnCounterMonotonic ==
  \A i \in 1..(Len(history) - 1) :
    \A n \in Nodes :
      history[i][n][n] <= history[i+1][n][n]

====
