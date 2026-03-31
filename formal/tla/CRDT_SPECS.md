# CRDT Specifications for RaisinDB

This document describes the TLA+ specifications for the CRDT (Conflict-free Replicated Data Type) semantics used in RaisinDB's distributed replication system.

## Overview

RaisinDB uses four main CRDT types to ensure eventual consistency across distributed nodes:

1. **LWW (Last-Write-Wins)** - For single-valued properties
2. **Add-Wins Set** - For relations between nodes
3. **Delete-Wins** - For node deletion semantics
4. **RGA (Replicated Growable Array)** - For ordered lists

## Specifications

### 1. LWW.tla - Last-Write-Wins CRDT

**Purpose**: Models single-valued properties where the write with the latest timestamp wins.

**Rust Implementation Mapping**:
- `crates/raisin-rocksdb/src/replication/application.rs` (lines 1460-1499)
  - `apply_upsert_node_snapshot()`: Applies LWW semantics using HLC revision
- `crates/raisin-hlc/src/lib.rs` (lines 225-232)
  - `impl Ord for HLC`: Timestamp ordering (timestamp first, then counter)

**Key Properties Verified**:
- **Determinism**: Same set of operations always produces the same winner
- **Commutativity**: `Merge(a,b) = Merge(b,a)`
- **Associativity**: `Merge(Merge(a,b),c) = Merge(a,Merge(b,c))`
- **Convergence**: All replicas converge to the same state
- **Monotonicity**: Once a node adopts an operation, it never adopts an older one

**How to Run**:
```bash
cd formal/tla
make LWW
```

**Configuration**:
- 3 nodes
- 2 node IDs
- Max timestamp: 3, Max counter: 2
- Max 6 operations for bounded model checking

---

### 2. AddWinsSet.tla - Add-Wins Set CRDT

**Purpose**: Models relations where concurrent add and remove operations resolve with add winning.

**Rust Implementation Mapping**:
- `crates/raisin-rocksdb/src/replication/application.rs`
  - Lines 2437-2505: `apply_add_relation()`
  - Lines 2508-2572: `apply_remove_relation()`
- `crates/raisin-replication/src/operation.rs` (lines 181-199)
  - `AddRelation` / `RemoveRelation` operations
  - Line 187: `relation_id` (UUID for Add-Wins semantics)

**Key Semantic**:
A relation exists if there's an add operation that is NOT causally preceded by a remove. This means:
- Concurrent add + remove → relation EXISTS (Add-Wins)
- Remove only takes effect if it happens-after the add

**Key Properties Verified**:
- **Add-Wins**: Concurrent add/remove → element exists
- **Convergence**: Same operations → same final set
- **Commutativity**: Order of applying operations doesn't matter
- **Remove Causality**: Remove only removes if it causally follows add
- **Eventual Consistency**: All nodes converge once all operations delivered

**How to Run**:
```bash
cd formal/tla
make AddWinsSet
```

**Configuration**:
- 3 nodes
- 2 relation IDs
- Max 8 operations

---

### 3. DeleteWins.tla - Delete-Wins Semantics

**Purpose**: Models node deletion where deletes always take precedence (no resurrection).

**Rust Implementation Mapping**:
- `crates/raisin-rocksdb/src/replication/application.rs`
  - Lines 1501-1559: `apply_delete_node_snapshot()`
  - Line 1503: "Delete-Wins semantics - deletions always take precedence"
  - Lines 1542-1552: Writes tombstones with revision HLC
- `crates/raisin-replication/src/operation.rs` (lines 217-230)
  - `UpsertNodeSnapshot` / `DeleteNodeSnapshot` operations

**Key Semantic**:
A node is DELETED if there exists a delete operation and all upsert operations either:
- Have earlier revision HLC, OR
- Are concurrent with delete (Delete-Wins)

This ensures:
- Concurrent upsert + delete → node DELETED
- Once deleted with a certain revision, earlier upserts cannot resurrect it

**Key Properties Verified**:
- **Delete-Wins**: Concurrent upsert/delete → node deleted
- **No Resurrection**: Once deleted, stays deleted (unless newer upsert)
- **Convergence**: Same operations → same final state
- **Revision Monotonicity**: Delete revision ≥ any upsert revision for deleted nodes
- **Tombstone Persistence**: Deletes create persistent markers

**How to Run**:
```bash
cd formal/tla
make DeleteWins
```

**Configuration**:
- 2 nodes
- 2 node IDs
- Max timestamp: 3, Max counter: 1
- Max 8 operations

---

### 4. RGA.tla - Replicated Growable Array

**Purpose**: Models ordered lists with concurrent insertions and deletions.

**Rust Implementation Mapping**:
- `crates/raisin-replication/src/operation.rs`
  - Lines 232-250: `ListInsertAfter` operation
  - Lines 242-250: `ListDelete` operation
  - Line 239: `element_id` (unique immutable ID for each list element)

**Key Concepts**:
- Each element has a unique immutable ID
- Elements reference the ID they were inserted after
- Deletions are tombstones (mark as deleted, don't remove)
- Ordering is determined by causal position references
- Uses vector clocks for causal ordering

**Key Properties Verified**:
- **Causal Order**: Insertions respect happens-before relation
- **Tombstone Correctness**: Deleted elements remain but are invisible
- **Convergence**: Same operations → same visible list
- **No Duplicate IDs**: Each element ID appears at most once
- **Insert Idempotency**: Same insert has same effect
- **Valid References**: All `afterId` references point to valid elements

**How to Run**:
```bash
cd formal/tla
make RGA
```

**Configuration**:
- 2 nodes
- 3 element IDs
- 3 values
- Max 8 operations

---

## Running All CRDT Specs

To check all CRDT specifications:

```bash
cd formal/tla
make all
```

This will run model checking on all 8 specifications (including the 4 CRDT specs).

## Quick Syntax Check

For a fast syntax-only check without full model checking:

```bash
cd formal/tla
make syntax
```

## Understanding the Code Mapping

Each TLA+ specification includes detailed comments mapping to the Rust implementation:

### Example from LWW.tla:
```tla
(***************************************************************************
 * Implementation Mapping:
 * - Rust: crates/raisin-rocksdb/src/replication/application.rs
 *   - Lines 1460-1499: apply_upsert_node_snapshot
 *   - Lines 1462-1463: "LWW semantics using revision HLC"
 ***************************************************************************)
```

### Example from AddWinsSet.tla:
```tla
(***************************************************************************
 * A relation with UUID 'u' EXISTS if and only if:
 * - There exists an ADD operation for 'u', AND
 * - For all REMOVE operations for 'u', the remove does NOT causally follow
 *   the add (i.e., either remove is concurrent or happened before add)
 ***************************************************************************)
```

## Key Invariants Checked

All CRDT specs verify the following core invariants:

1. **TypeOK**: All variables maintain correct types
2. **Convergence**: Nodes with same operations have same state
3. **Eventual Consistency**: All nodes converge when all operations delivered
4. **Commutativity**: Operation application order doesn't affect final state

## Example Concurrent Scenarios

Each specification includes example scenarios demonstrating concurrent operations:

### LWW - Concurrent Writes
```
Node1 writes "v1" at HLC(100, 0)
Node2 writes "v2" at HLC(101, 0)
Expected: "v2" wins (later timestamp)
```

### AddWinsSet - Concurrent Add/Remove
```
Node1 adds relation with VC {node1:1}
Node2 removes same relation with VC {node2:1}
VCs are concurrent → relation EXISTS (Add-Wins)
```

### DeleteWins - Concurrent Upsert/Delete
```
Node1 upserts with revision (100, 0)
Node2 deletes with revision (100, 0)
Result: Node DELETED (Delete-Wins)
```

### RGA - Concurrent Inserts
```
Node1 inserts A after head with VC {node1:1}
Node2 inserts B after head with VC {node2:1}
Both appear, order determined by VC comparison
```

## State Space Considerations

The model checking is bounded to keep state space manageable:

- **Small node counts** (2-3 nodes): Captures concurrency patterns
- **Limited operations** (6-8 ops): Tests core conflict scenarios
- **Bounded timestamps/counters**: Focuses on ordering logic
- **Symmetry reduction**: Uses node/ID permutations to reduce states

For production use, the properties hold for unbounded parameters due to CRDT mathematical properties.

## Verification Workflow

1. **Write Rust implementation** with CRDT semantics
2. **Model in TLA+** capturing core conflict resolution rules
3. **Model check** to verify properties (determinism, convergence, etc.)
4. **Document mapping** between TLA+ spec and Rust code
5. **Iterate** if bugs found

## References

- **CRDT Survey**: Shapiro et al., "Conflict-free Replicated Data Types" (2011)
- **RGA Paper**: Roh et al., "Replicated abstract data types" (2011)
- **HLC Paper**: Kulkarni et al., "Logical Physical Clocks" (2014)
- **TLA+ Book**: Lamport, "Specifying Systems" (2002)

## Files Created

### Specifications
- `/formal/tla/LWW.tla` - Last-Write-Wins CRDT
- `/formal/tla/AddWinsSet.tla` - Add-Wins Set CRDT
- `/formal/tla/DeleteWins.tla` - Delete-Wins semantics
- `/formal/tla/RGA.tla` - Replicated Growable Array

### Configuration Files
- `/formal/tla/LWW.cfg`
- `/formal/tla/AddWinsSet.cfg`
- `/formal/tla/DeleteWins.cfg`
- `/formal/tla/RGA.cfg`

### Build System
- `/formal/tla/Makefile` - Updated with new targets

### Documentation
- `/formal/tla/CRDT_SPECS.md` - This file
