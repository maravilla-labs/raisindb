# Formal Verification of RaisinDB CRDT Replication System

This directory contains formal specifications and proofs for the RaisinDB distributed replication system.

## Overview

RaisinDB implements an operation-based CRDT replication system with:
- Vector clock-based causal ordering
- Causal delivery buffer ensuring proper operation sequencing
- Persistent idempotency tracking preventing duplicates
- Multiple CRDT semantics: LWW (properties), Add-Wins (relations), Delete-Wins (deletes), RGA (lists)

## Verification Approach

We use a **hybrid formal verification strategy**:

1. **TLA+ for model checking** (primary tool)
   - Express system as state machines
   - Verify properties using TLC model checker
   - Generate counterexamples for violations

2. **Property-based testing** (continuous validation)
   - QuickCheck/PropTest in Rust
   - Generate random operation sequences
   - Verify properties hold

3. **Isabelle/HOL for theorem proving** (future)
   - Mathematical proofs of convergence
   - Unbounded state space guarantees

## Directory Structure

```
formal/
├── README.md                 # This file
├── tla/                      # TLA+ specifications
│   ├── NetworkModel.tla      # Asynchronous network model
│   ├── VectorClock.tla       # Vector clock semantics
│   ├── CausalDelivery.tla    # Causal delivery buffer
│   ├── Idempotency.tla       # Idempotency tracker
│   ├── LWW.tla               # Last-Write-Wins CRDT
│   ├── AddWinsSet.tla        # Add-Wins Set CRDT
│   ├── DeleteWins.tla        # Delete-Wins semantics
│   ├── RGA.tla               # Replicated Growable Array
│   ├── RaisinReplication.tla # Main specification
│   └── MCRaisinReplication.tla # Model checking config
├── isabelle/                 # Isabelle/HOL proofs (future)
│   └── RaisinCRDT.thy
├── docs/                     # Documentation
│   ├── PROPERTIES.md         # Verified properties
│   ├── MAPPING.md            # Spec ↔ Code mapping
│   └── SETUP.md              # Tool installation guide
└── tests/                    # Generated test cases
    └── traces/               # TLA+ execution traces
```

## Key Properties Verified

### Safety Properties

1. **Strong Eventual Consistency (SEC)**
   - Replicas that have delivered the same set of operations have equivalent state
   - Formalized in `RaisinReplication.tla`

2. **Causal Consistency**
   - If operation A causally precedes operation B, then A is delivered before B on all replicas
   - Formalized in `CausalDelivery.tla`

3. **Idempotency**
   - Applying the same operation multiple times has the same effect as applying it once
   - Formalized in `Idempotency.tla`

4. **Commutativity**
   - Concurrent operations (not causally related) can be applied in any order with the same result
   - Proven for each CRDT type (LWW, Add-Wins, Delete-Wins, RGA)

### Liveness Properties

1. **Eventual Delivery**
   - Every operation sent is eventually delivered to all correct replicas
   - Assumes eventual message delivery (no permanent network partitions)

2. **Progress**
   - The system makes forward progress (operations don't remain buffered indefinitely)
   - Formalized as temporal properties in TLA+

## Getting Started

### Prerequisites

1. **Install TLA+ Tools**:
   ```bash
   # Download TLA+ Tools
   wget https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar

   # Or use the TLA+ Toolbox (GUI)
   # https://github.com/tlaplus/tlaplus/releases
   ```

2. **Install Java** (required for TLA+ Tools):
   ```bash
   # macOS
   brew install openjdk@17

   # Ubuntu/Debian
   sudo apt-get install openjdk-17-jdk
   ```

3. **Optional: Install VS Code extension**:
   - Extension: "TLA+" by alygin
   - Provides syntax highlighting and integration with TLC

### Running Model Checker

```bash
# Check a specification
java -jar tla2tools.jar formal/tla/VectorClock.tla

# Run with specific model checking config
java -jar tla2tools.jar formal/tla/MCRaisinReplication.tla

# With more workers for parallelism
java -jar tla2tools.jar -workers 8 formal/tla/MCRaisinReplication.tla

# Generate coverage statistics
java -jar tla2tools.jar -coverage 1 formal/tla/RaisinReplication.tla
```

### Running Property-Based Tests

```bash
# Run all property tests
cd /path/to/raisindb
cargo test --test property_tests -- --nocapture

# Run specific property test
cargo test --test property_tests prop_strong_eventual_consistency

# Run with more test cases
cargo test --test property_tests -- --nocapture QUICKCHECK_TESTS=10000
```

## Verification Status

| Component | TLA+ Spec | Model Checked | Property Tests | Isabelle Proof |
|-----------|-----------|---------------|----------------|----------------|
| Network Model | ✅ | ✅ | N/A | ⏳ |
| Vector Clock | ✅ | ✅ | ✅ | ⏳ |
| Causal Delivery | ✅ | ✅ | ✅ | ⏳ |
| Idempotency | ✅ | ✅ | ✅ | ⏳ |
| LWW CRDT | ✅ | ✅ | ✅ | ⏳ |
| Add-Wins Set | ✅ | ✅ | ✅ | ⏳ |
| Delete-Wins | ✅ | ✅ | ✅ | ⏳ |
| RGA | ✅ | ✅ | ✅ | ⏳ |

Legend:
- ✅ Complete
- 🚧 In Progress
- ⏳ Planned
- ❌ Not Started

## References

### Academic Papers

1. **Gomes, Kleppmann et al. (2017)**
   - "Verifying Strong Eventual Consistency in Distributed Systems"
   - OOPSLA 2017
   - https://github.com/trvedata/crdt-isabelle

2. **Burckhardt et al. (2014)**
   - "Replicated Data Types: Specification, Verification, Optimality"
   - POPL 2014

3. **Shapiro et al. (2011)**
   - "Conflict-free Replicated Data Types"
   - Technical Report, INRIA

### Industry Applications

1. **AWS TLA+ Case Studies**
   - S3, DynamoDB replication protocols
   - https://lamport.azurewebsites.net/tla/amazon.html

2. **Microsoft Cosmos DB**
   - Formal verification of consistency models
   - https://www.microsoft.com/en-us/research/publication/azure-cosmos-db/

## Contributing

When adding new operations or CRDT types:

1. **Update TLA+ specs** in `formal/tla/`
2. **Run model checker** to verify properties still hold
3. **Add property tests** in `crates/raisin-replication/tests/property_tests.rs`
4. **Update documentation** in `formal/docs/MAPPING.md`

## License

Same as RaisinDB project (see root LICENSE file).
