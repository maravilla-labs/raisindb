# CI/CD Integration for Formal Verification

This document explains how formal verification is integrated into the CI/CD pipeline for RaisinDB.

## Overview

The formal verification system runs automatically on every push and pull request, providing continuous validation of CRDT correctness properties.

## GitHub Actions Workflow

**Location:** `.github/workflows/formal-verification.yml`

### Jobs

The workflow consists of 4 jobs:

#### 1. Property-Based Tests
- **Runtime:** ~2-5 minutes
- **Runs:** 100 test cases per property (1000 on PRs)
- **Tests:** 14 properties covering SEC, idempotency, commutativity, etc.
- **Language:** Rust (using `proptest`)

#### 2. TLA+ Model Checking
- **Runtime:** ~15-25 minutes
- **Runs:** TLC model checker on 8 TLA+ specifications
- **Verifies:** 32+ invariants and temporal properties
- **Critical:** CausalDelivery.tla (ensures CRDT convergence)

#### 3. Multi-Node Integration Tests
- **Runtime:** ~5-10 minutes
- **Runs:** 4 integration tests with actual RocksDB instances
- **Tests:** Convergence, partitions, crash recovery, out-of-order delivery

#### 4. Verification Summary
- **Runtime:** <1 minute
- **Aggregates:** Results from all jobs
- **Reports:** Pass/fail status and verified properties

## Triggers

The workflow runs on:
- **Push** to `main`, `develop`, or `feature/*` branches
- **Pull requests** to `main` or `develop`
- **Manual trigger** via GitHub Actions UI

## What Gets Verified

### CRDT Properties (Property-Based Tests)

✅ **Strong Eventual Consistency**
- Replicas with same operations converge to equivalent state

✅ **Idempotency**
- Applying operation twice = applying once

✅ **Commutativity**
- Concurrent operations can be applied in any order

✅ **Causal Consistency**
- Causally-related operations delivered in order

### TLA+ Specifications

| Spec | Invariants | Liveness | Runtime |
|------|------------|----------|---------|
| VectorClock | 5 | - | ~2 min |
| NetworkModel | 3 | 1 | ~3 min |
| Idempotency | 4 | - | ~2 min |
| **CausalDelivery** | **5** | **1** | **~5 min** |
| LWW | 6 | - | ~3 min |
| AddWinsSet | 5 | 1 | ~4 min |
| DeleteWins | 4 | - | ~3 min |
| RGA | 5 | 1 | ~4 min |

**Total:** 37 invariants, 4 liveness properties

### Integration Tests

1. **Multi-Node Convergence** - 3 nodes in full mesh
2. **Persistent Idempotency** - Operations not reapplied
3. **Network Partition** - Convergence after healing
4. **Out-of-Order Delivery** - Causal buffer works correctly

## Running Locally

### Property-Based Tests

```bash
# Run all property tests
cargo test --package raisin-replication --test property_tests

# Run with more test cases
PROPTEST_CASES=1000 cargo test --package raisin-replication --test property_tests
```

### TLA+ Model Checking

```bash
# Prerequisites
brew install openjdk@17
wget https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar

# Run all specs
cd formal/tla
make all

# Run specific spec
make CausalDelivery
```

### Integration Tests

```bash
# Run multi-node CRDT tests
cargo test --package raisin-rocksdb --test multi_node_crdt_integration
```

## CI/CD Performance

| Job | Typical Runtime | Cache Hit | First Run |
|-----|-----------------|-----------|-----------|
| Property Tests | 2-3 min | 1-2 min | 5-7 min |
| TLA+ Checking | 15-20 min | 15-20 min | 15-20 min |
| Integration Tests | 5-8 min | 3-5 min | 10-15 min |
| **Total** | **22-31 min** | **19-27 min** | **30-42 min** |

## Interpreting Results

### Success (✅)
All jobs passed - CRDT properties verified:
```
✅ Property-Based Tests: PASSED
✅ TLA+ Model Checking: PASSED
✅ Integration Tests: PASSED
```

### Failure (❌)

**Property Test Failure:**
- Indicates property violation
- Check logs for failing test case
- PropTest provides minimal counterexample
- Review shrunk test case for root cause

**TLA+ Model Checking Failure:**
- Invariant violation or deadlock detected
- Download TLC logs from artifacts
- Review error trace
- Check state leading to violation

**Integration Test Failure:**
- Actual implementation issue
- Check test logs for failure details
- May indicate code/spec mismatch

## Artifacts

The workflow uploads artifacts on completion:

**TLC Logs** (retained 7 days):
- Individual spec logs
- State exploration statistics
- Error traces (if violations found)

## Optimization Tips

### Faster Local Runs

```bash
# Run only fast checks
cargo test --package raisin-replication --test property_tests -- --test-threads=4

# Model check with fewer workers
cd formal/tla && tlc -workers 2 VectorClock.tla
```

### Parallel Model Checking

The workflow runs all TLA+ specs in parallel:
- VectorClock, NetworkModel (fast)
- Idempotency, LWW (medium)
- CausalDelivery, AddWinsSet, DeleteWins, RGA (slower)

## Debugging Failures

### Property Test Failure

1. **Locate the failing property:**
   ```
   thread 'prop_strong_eventual_consistency' panicked at...
   ```

2. **Check the seed** (for reproducibility):
   ```
   proptest: test failed at SEED=XYZ
   ```

3. **Rerun with seed:**
   ```bash
   PROPTEST_CASES=1 PROPTEST_REPLAY=XYZ cargo test ...
   ```

4. **Review minimal counterexample:**
   PropTest automatically shrinks to smallest failing case

### TLA+ Failure

1. **Download TLC logs** from GitHub Actions artifacts

2. **Review error trace:**
   ```
   Error: Invariant CausalOrderInvariant is violated.
   State 1: ...
   State 2: ...
   State 3: ...  <-- Invariant violation
   ```

3. **Analyze states:**
   - What changed between states?
   - Which operation caused violation?
   - Is it a spec error or implementation bug?

4. **Fix and rerun:**
   - Update TLA+ spec OR
   - Fix Rust implementation

### Integration Test Failure

1. **Check test logs** for assertion failure:
   ```
   assertion failed: node1_ops.len() == node2_ops.len()
   ```

2. **Enable detailed logging:**
   ```bash
   RUST_LOG=debug cargo test --test multi_node_crdt_integration -- --nocapture
   ```

3. **Review operation logs:**
   - Which operations were applied?
   - What was the vector clock state?
   - Did catch-up trigger?

## Maintenance

### Adding New Properties

1. **Add property test** to `tests/property_tests.rs`
2. **Run locally** to verify
3. **Push** - CI runs automatically

### Adding New TLA+ Specs

1. **Create spec** in `formal/tla/NewSpec.tla`
2. **Create config** in `formal/tla/NewSpec.cfg`
3. **Update Makefile** with new target
4. **Add step** to `.github/workflows/formal-verification.yml`

### Updating Constants

Edit `.cfg` files to adjust model checking bounds:

```tla
\* NewSpec.cfg
CONSTANTS
  Nodes = {n1, n2, n3}
  MaxOps = 5   \* Increase for deeper exploration (slower)
```

## Best Practices

1. **Run locally before pushing:**
   ```bash
   cargo test --package raisin-replication --test property_tests
   cd formal/tla && make all
   ```

2. **Monitor CI duration:**
   - If >30 minutes, consider optimization
   - Reduce TLA+ constants if needed

3. **Review failures immediately:**
   - CRDT bugs are subtle and hard to debug later
   - Fix before merging to main

4. **Update specs with code changes:**
   - Modified operation types? Update TLA+
   - New CRDT semantics? Add spec
   - Changed conflict resolution? Verify in TLA+

## Resources

- **GitHub Actions Workflow:** `.github/workflows/formal-verification.yml`
- **Property Tests:** `crates/raisin-replication/tests/property_tests.rs`
- **TLA+ Specs:** `formal/tla/*.tla`
- **Integration Tests:** `crates/raisin-rocksdb/tests/multi_node_crdt_integration.rs`

## Troubleshooting

**"TLA+ Tools not found"**
- Check Java 17+ is installed
- Verify tla2tools.jar download URL

**"Property tests timeout"**
- Reduce PROPTEST_CASES
- Check for infinite loops in generators

**"Model checking takes too long"**
- Reduce MaxOps in .cfg files
- Use symmetry reduction
- Run with fewer workers

**"Integration tests flaky"**
- Check for timing dependencies
- Increase wait timeouts
- Review TCP port conflicts

## Summary

The CI/CD integration provides:
- **Continuous verification** on every commit
- **Automated property checking** (14 properties)
- **Model checking** (8 specs, 37 invariants)
- **Integration testing** (4 multi-node scenarios)
- **Fast feedback** (~20-30 minutes total)

This ensures the CRDT replication system maintains correctness as the codebase evolves.
