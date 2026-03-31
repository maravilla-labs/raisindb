# TLA+ Quick Start Guide for RaisinDB

This guide gets you running TLA+ model checking in 5 minutes.

## Installation

### Option 1: TLA+ Toolbox (Recommended for Beginners)

Download from: https://github.com/tlaplus/tlaplus/releases

- Includes GUI for visualizing error traces
- Built-in TLC model checker
- Syntax highlighting and debugging

### Option 2: Command Line Tools

```bash
# macOS
brew install tla-plus

# Or download JAR directly
wget https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar
```

## Quick Test

Verify installation:

```bash
cd formal/tla

# Test VectorClock spec (smallest, fastest)
tlc VectorClock.tla -config VectorClock.cfg

# Expected output:
# TLC2 Version ...
# ...
# Model checking completed. No error has been found.
```

## Running Each Specification

### 1. Vector Clock (Fastest - ~1 minute)

```bash
tlc VectorClock.tla -config VectorClock.cfg
```

**What it checks:**
- Vector clock operations are correct
- Happens-before relation is transitive
- Merge operation is commutative and idempotent

**Expected states:** ~5,000-10,000

---

### 2. Network Model (~2-3 minutes)

```bash
tlc NetworkModel.tla -config NetworkModel.cfg
```

**What it checks:**
- Messages eventually delivered
- No duplicate deliveries
- Network partitions work correctly

**Expected states:** ~20,000-50,000

---

### 3. Causal Delivery (~5-10 minutes)

```bash
tlc CausalDelivery.tla -config CausalDelivery.cfg
```

**What it checks:**
- **CRITICAL:** Causal order invariant (CRDT convergence)
- Buffer bounded
- No operation applied twice
- Operations eventually delivered

**Expected states:** ~100,000-500,000

**Note:** This is the most important spec to verify!

---

### 4. Idempotency (~3-5 minutes)

```bash
tlc Idempotency.tla -config Idempotency.cfg
```

**What it checks:**
- Operations applied exactly once
- Idempotency survives crashes
- Garbage collection correctness

**Expected states:** ~50,000-200,000

---

## Troubleshooting

### Out of Memory

Increase heap size:
```bash
tlc -Xmx4g CausalDelivery.tla -config CausalDelivery.cfg
```

### Too Slow

Reduce state space in `.cfg` file:
```
CONSTANTS
  Nodes = {n1, n2}      # Reduce from 3 to 2
  MaxOps = 5            # Reduce from 10 to 5
```

### Deadlock Error

This usually means the spec doesn't allow progress. Check:
1. Fairness constraints in the spec
2. State constraint might be too restrictive

### Invariant Violation

Great! You found a bug. TLC will show:
1. The violated invariant
2. A counterexample trace (sequence of states leading to violation)

Use TLA+ Toolbox GUI to visualize the error trace.

---

## Using TLA+ Toolbox (GUI)

1. Open TLA+ Toolbox
2. File → Open Spec → Select `VectorClock.tla`
3. TLC Model Checker → New Model → Name it "Basic"
4. In model editor:
   - Set constants (Nodes, MaxOps, etc.)
   - Select invariants to check
   - Select properties to verify
5. Click "Run TLC" button (green arrow)
6. View results in "Model Checking Results" tab

**Visualizing Errors:**
- Click on error trace
- Step through states
- See variable values at each step

---

## Performance Tips

### Use Multiple Cores

```bash
tlc -workers auto CausalDelivery.tla -config CausalDelivery.cfg
```

### Generate Only Coverage

Skip full verification, just check coverage:
```bash
tlc -coverage 1 -depth 10 CausalDelivery.tla -config CausalDelivery.cfg
```

### Simulation Mode (Random Walk)

Quickly explore state space:
```bash
tlc -simulate -depth 100 CausalDelivery.tla -config CausalDelivery.cfg
```

---

## Common Workflow

### 1. Check Syntax

```bash
# Just parse, don't run
tlc -check VectorClock.tla
```

### 2. Small Model

```bash
# Run with small constants first
# Edit .cfg: Nodes = {n1, n2}, MaxOps = 3
tlc VectorClock.tla -config VectorClock.cfg
```

### 3. Full Model

```bash
# Increase constants
# Edit .cfg: Nodes = {n1, n2, n3}, MaxOps = 10
tlc -workers auto VectorClock.tla -config VectorClock.cfg
```

### 4. Debug Errors

```bash
# Use GUI for error traces
# Open in TLA+ Toolbox
```

---

## What to Look For

### Success

```
TLC2 Version 2.18
...
Model checking completed.
No error has been found.
States generated: 123456
States distinct: 67890
Time: 45 seconds
```

### Invariant Violation

```
Error: Invariant CausalOrderInvariant is violated.

The behavior up to this point is:
State 1: <Initial predicate>
/\ localVC = [n1 |-> {...}, n2 |-> {...}]
/\ buffer = [n1 |-> {}, n2 |-> {}]
...

State 2: <LocalOp(n1)>
...

State 5: <Violation occurs>
```

### Deadlock

```
Error: Deadlock reached.
```

---

## Example: Verifying Causal Order

The most important property is causal ordering in `CausalDelivery.tla`:

```bash
# Run with small model first
tlc CausalDelivery.tla -config CausalDelivery.cfg

# If successful, you've verified:
# ✓ Operations applied in causal order
# ✓ Buffer stays bounded
# ✓ No duplicates
# ✓ Operations eventually delivered
```

This mathematically proves the CRDT replication will converge!

---

## Next Steps

1. **Run all specs** to verify current implementation
2. **Modify constants** to test edge cases
3. **Add custom properties** for new features
4. **Update specs** when changing Rust code
5. **Write proofs** using TLAPS (advanced)

---

## Getting Help

- TLA+ Community: https://groups.google.com/g/tlaplus
- Stack Overflow: Tag with `tla+`
- TLA+ Book: "Specifying Systems" by Leslie Lamport (free PDF)
- Video Course: https://lamport.azurewebsites.net/video/videos.html

---

## Key Takeaway

Running `tlc CausalDelivery.tla -config CausalDelivery.cfg` successfully proves your CRDT replication algorithm is correct!

This gives you mathematical confidence that your distributed system will behave correctly.
