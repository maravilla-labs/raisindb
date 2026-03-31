# TLA+ Specifications for RaisinDB CRDT Replication

This directory contains formal TLA+ specifications for the RaisinDB CRDT replication system. These specifications model the core algorithms and verify their correctness properties.

## Specifications

### 1. NetworkModel.tla - Asynchronous Network Model

Models a realistic distributed network with:
- **Arbitrary message delays**: Messages can be delayed by 0 to `MaxDelay` time units
- **Message reordering**: Messages can arrive out of order
- **Eventual delivery**: All messages are eventually delivered (no permanent loss)
- **Network partitions**: Simulates network splits and healing

**Key Invariants:**
- `NoStuckMessages`: All messages eventually become deliverable
- `NoDuplicateMessages`: No duplicate messages in the network
- `BoundedNetwork`: Network size stays within bounds
- `NoDoubleDelivery`: Messages delivered at most once per node

**Key Properties:**
- `EventualDelivery`: Messages eventually reach their destination (unless partitioned)

**Based on:** Asynchronous network model used throughout the replication system

---

### 2. VectorClock.tla - Vector Clock Semantics

Formalizes vector clocks and their mathematical properties:
- **Increment**: Local operation advances node's counter
- **Merge**: Pointwise maximum of two vector clocks
- **Happens-before**: Partial order relation (vc1 < vc2)
- **Concurrent**: Neither happens-before nor happens-after
- **Distance**: Replication lag metric

**Key Invariants:**
- `IrreflexiveHappensBefore`: vc < vc is always false
- `AntisymmetricHappensBefore`: If vc1 < vc2, then NOT vc2 < vc1
- `TransitiveHappensBefore`: If vc1 < vc2 and vc2 < vc3, then vc1 < vc3
- `MergeCommutative`: Merge(vc1, vc2) = Merge(vc2, vc1)
- `MergeIdempotent`: Merge(vc, vc) = vc
- `MonotonicClocks`: Vector clocks only increase over time

**Key Theorems:**
- Vector clocks form a strict partial order
- Merge creates a least upper bound
- Increment preserves ordering

**Based on:** `crates/raisin-replication/src/vector_clock.rs`

---

### 3. CausalDelivery.tla - Causal Delivery Buffer

Models the causal delivery buffer that ensures operations are applied only when their causal dependencies are satisfied:

- **Buffer management**: Operations buffered until dependencies satisfied
- **Dependency checking**: Verifies vector clock causality
- **Cascading delivery**: Buffered ops delivered when dependencies arrive
- **Bounded buffer**: Prevents unbounded memory growth

**Key Invariants:**
- `CausalOrderInvariant`: **CRITICAL** - If op1 happens-before op2, then op1 applied before op2
- `BufferBounded`: Buffer never exceeds `MaxBufferSize`
- `NoDoubleApplication`: Operations applied at most once per node
- `BufferedOpsWaiting`: All buffered operations have unsatisfied dependencies
- `SameNodeSequenceOrder`: Operations from same node delivered in sequence

**Key Properties:**
- `EventualDelivery`: All operations eventually delivered (liveness)
- `BufferEventuallyDrains`: Buffer doesn't grow unbounded

**Why This Matters:**
Operation-based CRDTs **REQUIRE** causal delivery for convergence. Without it, operations could apply before their dependencies, causing permanent state divergence.

**Based on:** `crates/raisin-replication/src/causal_delivery.rs`

---

### 4. Idempotency.tla - Idempotency Tracking

Models the persistent idempotency tracker that ensures operations are applied exactly once:

- **Applied operation tracking**: Persistent set of applied operation IDs
- **Timestamp recording**: Tracks when operations were applied (for GC)
- **Crash recovery**: Verifies persistence across node restarts
- **Garbage collection**: Removes old operation IDs after TTL

**Key Invariants:**
- `IdempotencyInvariant`: Applying operation twice = applying once
- `NoFalseNegatives`: Applied operations always detected (unless GC'd)
- `TimestampsMonotonic`: Timestamps increase over time
- `GCCorrectness`: GC only removes operations older than TTL
- `NoDoubleApplication`: Deduplication works correctly

**Key Properties:**
- `EventualApplication`: Operations eventually get applied
- `EventualGC`: Old operations eventually garbage collected
- `CrashRecoveryScenario`: Idempotency survives crashes

**Based on:** `crates/raisin-rocksdb/src/replication/persistent_idempotency.rs`

---

## Running the Model Checker

### Prerequisites

Install TLA+ tools:
```bash
# Download TLA+ Toolbox (includes TLC model checker)
# https://github.com/tlaplus/tlaplus/releases

# Or install via command line tools
brew install tla-plus/tlaplus/tlaps  # macOS
```

### Running TLC

Each specification has a corresponding `.cfg` file with model checking parameters.

#### Check VectorClock properties:
```bash
cd formal/tla
tlc VectorClock.tla -config VectorClock.cfg
```

#### Check CausalDelivery invariants:
```bash
tlc CausalDelivery.tla -config CausalDelivery.cfg
```

#### Check Idempotency with crash recovery:
```bash
tlc Idempotency.tla -config Idempotency.cfg
```

#### Check Network model:
```bash
tlc NetworkModel.tla -config NetworkModel.cfg
```

### Interpreting Results

**Success:**
```
TLC finished checking model.
No errors found.
States: 12345 (distinct: 6789)
```

**Invariant Violation:**
```
Error: Invariant CausalOrderInvariant is violated.
Counterexample trace:
  State 1: ...
  State 2: ...
```

**Deadlock:**
```
Error: Deadlock reached.
```
This usually means the spec doesn't allow any further progress - check fairness constraints.

---

## Customizing Model Checking

### Adjusting Constants

Edit the `.cfg` files to change model parameters:

```
CONSTANTS
  Nodes = {n1, n2, n3}      \* Increase for more nodes
  MaxOps = 10               \* Increase for longer traces
  MaxBufferSize = 5         \* Test different buffer sizes
```

**Warning:** Increasing these values exponentially increases state space!

### State Constraints

All specs include state constraints to prevent state explosion:
```tla
StateConstraint ==
  /\ opCounter <= MaxOps
  /\ \A node \in Nodes : Cardinality(buffer[node]) <= MaxBufferSize
```

### Adding Properties

Add custom temporal properties to `.cfg`:
```
PROPERTIES
  MyCustomProperty
```

Then define in `.tla`:
```tla
MyCustomProperty == <>[](\A n \in Nodes : buffer[n] = {})
```

---

## Model Checking Tips

1. **Start small**: Begin with 2 nodes, MaxOps=5
2. **Incremental verification**: Check one invariant at a time
3. **Use symmetry**: TLC can exploit symmetry in node IDs
4. **Monitor progress**: Use `-workers auto` for parallel checking
5. **Debugging**: Use TLA+ Toolbox GUI to visualize error traces

### Advanced TLC Options

```bash
# Use multiple cores
tlc -workers auto VectorClock.tla -config VectorClock.cfg

# Generate coverage statistics
tlc -coverage 1 CausalDelivery.tla -config CausalDelivery.cfg

# Deadlock checking
tlc -deadlock CausalDelivery.tla -config CausalDelivery.cfg

# Increase memory
tlc -Xmx4g Idempotency.tla -config Idempotency.cfg
```

---

## Proofs (Future Work)

These specifications can be extended with formal proofs using TLAPS (TLA+ Proof System):

```tla
THEOREM CausalOrderIsCorrect ==
  ASSUME Spec
  PROVE []CausalOrderInvariant
PROOF
  (* Proof steps would go here *)
```

---

## Integration with Rust Code

These formal specs are designed to mirror the Rust implementation:

| TLA+ Spec | Rust Module |
|-----------|-------------|
| `VectorClock.tla` | `raisin-replication/src/vector_clock.rs` |
| `CausalDelivery.tla` | `raisin-replication/src/causal_delivery.rs` |
| `Idempotency.tla` | `raisin-rocksdb/src/replication/persistent_idempotency.rs` |
| `NetworkModel.tla` | Used throughout replication system |

When modifying the Rust code, consider updating the TLA+ specs to verify correctness of changes.

---

## Common Verification Patterns

### Verifying Causal Order

The most critical property is `CausalOrderInvariant` in `CausalDelivery.tla`:

```tla
CausalOrderInvariant ==
  \A node \in Nodes :
    \A i, j \in 1..Len(applyOrder[node]) :
      (i < j) =>
        LET op1 == applyOrder[node][i]
            op2 == applyOrder[node][j]
        IN ~HappensBefore(op2.vectorClock, op1.vectorClock)
```

This ensures CRDT convergence.

### Verifying Idempotency

Test that operations can be applied multiple times safely:

```tla
IdempotencyInvariant ==
  \A n \in Nodes, op \in Operation :
    LET s1 == ApplyOp(state[n], op)
        s2 == ApplyOp(ApplyOp(state[n], op), op)
    IN s1 = s2
```

### Verifying Persistence

Ensure data survives crashes:

```tla
PersistenceCorrectness ==
  \A n \in Nodes, opId \in OpId :
    (opId \in appliedOps[n] /\ n \in crashed) =>
      opId \in appliedOps[n]  \* Still there after crash
```

---

## References

- [TLA+ Homepage](https://lamport.azurewebsites.net/tla/tla.html)
- [Learn TLA+](https://learntla.com/)
- [TLA+ Hyperbook](https://learntla.com/tla/)
- [TLA+ Examples](https://github.com/tlaplus/Examples)
- [Vector Clocks Paper](https://en.wikipedia.org/wiki/Vector_clock)
- [CRDT Research](https://crdt.tech/)

---

## Contributing

When adding new replication features:

1. **Write TLA+ spec first**: Model the algorithm formally
2. **Verify with TLC**: Check all invariants hold
3. **Implement in Rust**: Follow the verified design
4. **Test**: Unit tests should mirror TLA+ scenarios
5. **Update specs**: Keep TLA+ in sync with code changes

---

## License

These TLA+ specifications are part of RaisinDB and share the same license as the main project.
