# TLA+ ↔ Rust Code Mapping

This document maps the TLA+ CRDT specifications to their Rust implementations in RaisinDB.

## 1. LWW (Last-Write-Wins)

### TLA+ Specification
- **File**: `formal/tla/LWW.tla`
- **Lines**: 297 total
- **Key Concepts**:
  - HLC comparison (lines 56-62)
  - LWW merge logic (lines 93-97)
  - Determinism property (lines 175-178)

### Rust Implementation

#### HLC Data Structure
**Location**: `crates/raisin-hlc/src/lib.rs`
```rust
// Lines 62-68: HLC struct definition
pub struct HLC {
    pub timestamp_ms: u64,
    pub counter: u64,
}
```

**TLA+ Equivalent**: `LWW.tla` lines 43-46
```tla
HLC == [
  timestamp_ms: 0..MaxTimestamp,
  counter: 0..MaxCounter
]
```

#### HLC Ordering
**Location**: `crates/raisin-hlc/src/lib.rs`
```rust
// Lines 225-232: Ord implementation
impl Ord for HLC {
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestamp_ms
            .cmp(&other.timestamp_ms)
            .then_with(|| self.counter.cmp(&other.counter))
    }
}
```

**TLA+ Equivalent**: `LWW.tla` lines 56-62
```tla
HLCBefore(hlc1, hlc2) ==
  \/ hlc1.timestamp_ms < hlc2.timestamp_ms
  \/ (hlc1.timestamp_ms = hlc2.timestamp_ms /\ hlc1.counter < hlc2.counter)
```

#### LWW Application
**Location**: `crates/raisin-rocksdb/src/replication/application.rs`
```rust
// Lines 1460-1499: apply_upsert_node_snapshot
async fn apply_upsert_node_snapshot(
    &self,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,  // <-- LWW timestamp
    _op: &Operation,
) -> Result<()> {
    // Lines 1478-1490: Apply using revision HLC
    // The storage layer keeps versioned keys
    // load_latest_node returns version with highest revision (LWW)
    self.apply_replicated_upsert(
        tenant_id, repo_id, branch, workspace,
        node, parent_id, revision,
    )?;
}
```

**TLA+ Equivalent**: `LWW.tla` lines 93-97
```tla
LWWMerge(op1, op2) ==
  IF HLCBefore(op1.hlc, op2.hlc) THEN op2
  ELSE IF HLCAfter(op1.hlc, op2.hlc) THEN op1
  ELSE op1  \* Equal timestamps: deterministic tie-break
```

---

## 2. AddWinsSet (Add-Wins Set CRDT)

### TLA+ Specification
- **File**: `formal/tla/AddWinsSet.tla`
- **Lines**: 358 total
- **Key Concepts**:
  - Relation structure (lines 45-52)
  - Add-Wins semantics (lines 91-98)
  - Convergence property (lines 229-232)

### Rust Implementation

#### Relation Operations
**Location**: `crates/raisin-replication/src/operation.rs`
```rust
// Lines 181-199: AddRelation/RemoveRelation operations
OpType::AddRelation {
    source_id: String,
    relation_type: String,
    target_id: String,
    relation_id: Uuid,  // <-- Unique ID for Add-Wins semantics
    properties: HashMap<String, PropertyValue>,
}

OpType::RemoveRelation {
    source_id: String,
    relation_type: String,
    target_id: String,
    relation_id: Uuid,  // <-- Must match corresponding AddRelation
}
```

**TLA+ Equivalent**: `AddWinsSet.tla` lines 45-52
```tla
Relation == [
  fromNode: STRING,
  relationType: STRING,
  toNode: STRING,
  uuid: Relations,
  vc: VectorClock
]
```

#### Add Relation Implementation
**Location**: `crates/raisin-rocksdb/src/replication/application.rs`
```rust
// Lines 2437-2505: apply_add_relation
async fn apply_add_relation(
    &self,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_id: &str,
    relation_type: &str,
    target_id: &str,
    relation_id: uuid::Uuid,  // <-- Unique per relation instance
    properties: &HashMap<String, PropertyValue>,
    op: &Operation,
) -> Result<()> {
    let revision = Self::op_revision(op)?;

    // Write to forward index with versioned key
    let forward_key = keys::relation_forward_key_versioned(
        tenant_id, repo_id, branch, "default",
        source_id, relation_type, &revision, target_id,
    );

    self.db.put_cf(cf_relation, forward_key, &value)?;

    // Write to reverse index
    let reverse_key = keys::relation_reverse_key_versioned(
        tenant_id, repo_id, branch, "default",
        target_id, relation_type, &revision, source_id,
    );

    self.db.put_cf(cf_relation, reverse_key, &value)?;
}
```

**TLA+ Equivalent**: `AddWinsSet.tla` lines 118-135
```tla
AddRelation(node, fromNode, relType, toNode, uuid) ==
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
        /\ nodeState' = ... ComputeRelationSet ...
```

#### Add-Wins Semantics
**Location**: Implicit in the storage layer - relations exist if added and not causally removed
**Verification**: Done at read time by checking vector clocks

**TLA+ Formalization**: `AddWinsSet.tla` lines 91-98
```tla
RelationExists(relationId, addOps, removeOps) ==
  \E addOp \in addOps :
    /\ addOp.uuid = relationId
    /\ \A remOp \in removeOps :
        remOp.uuid = relationId =>
          \/ ~HappensBefore(addOp.vc, remOp.vc)  \* Add-Wins!
```

---

## 3. DeleteWins (Delete-Wins Semantics)

### TLA+ Specification
- **File**: `formal/tla/DeleteWins.tla`
- **Lines**: 387 total
- **Key Concepts**:
  - Node operations (lines 47-62)
  - Delete-Wins logic (lines 72-84)
  - No resurrection property (lines 241-250)

### Rust Implementation

#### Node Snapshot Operations
**Location**: `crates/raisin-replication/src/operation.rs`
```rust
// Lines 217-230: UpsertNodeSnapshot/DeleteNodeSnapshot
OpType::UpsertNodeSnapshot {
    node: Node,
    parent_id: Option<String>,
    revision: HLC,  // <-- Timestamp for conflict resolution
}

OpType::DeleteNodeSnapshot {
    node_id: String,
    revision: HLC,  // <-- Timestamp for conflict resolution
}
```

**TLA+ Equivalent**: `DeleteWins.tla` lines 47-62
```tla
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
```

#### Delete Node Implementation
**Location**: `crates/raisin-rocksdb/src/replication/application.rs`
```rust
// Lines 1501-1559: apply_delete_node_snapshot
async fn apply_delete_node_snapshot(
    &self,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    revision: &HLC,  // <-- Delete timestamp
    _op: &Operation,
) -> Result<()> {
    // Lines 1515-1526: Load node to get full information
    let node = match self.load_latest_node(tenant_id, repo_id, branch, node_id)? {
        Some(n) => n,
        None => {
            // Node doesn't exist, nothing to delete (idempotent)
            return Ok(());
        }
    };

    // Lines 1542-1552: Write tombstones with revision HLC
    // This implements Delete-Wins: tombstone persists
    self.apply_replicated_delete(
        tenant_id, repo_id, branch, workspace,
        &node, parent_id, revision,
    )?;
}
```

**TLA+ Equivalent**: `DeleteWins.tla` lines 72-84
```tla
NodeDeleted(nodeId, ops) ==
  \E delOp \in ops :
    /\ delOp.opType = "DELETE"
    /\ delOp.nodeId = nodeId
    /\ \A upsertOp \in ops :
        (upsertOp.nodeId = nodeId /\ upsertOp.opType = "UPSERT") =>
          \/ HLCBefore(upsertOp.revision, delOp.revision)
          \/ HLCEqual(upsertOp.revision, delOp.revision)  \* Delete-Wins!
```

**Key Insight**: Delete-Wins is enforced by comparing revision HLCs. If delete revision ≥ upsert revision, the node is deleted.

---

## 4. RGA (Replicated Growable Array)

### TLA+ Specification
- **File**: `formal/tla/RGA.tla`
- **Lines**: 406 total
- **Key Concepts**:
  - RGA element structure (lines 40-48)
  - Insert operation (lines 161-185)
  - Delete as tombstone (lines 187-209)

### Rust Implementation

#### List Operations
**Location**: `crates/raisin-replication/src/operation.rs`
```rust
// Lines 232-250: ListInsertAfter/ListDelete operations
OpType::ListInsertAfter {
    node_id: String,
    list_property: String,
    after_id: Option<Uuid>,  // <-- Insert after this element (None = head)
    value: PropertyValue,
    element_id: Uuid,  // <-- Unique immutable ID
}

OpType::ListDelete {
    node_id: String,
    list_property: String,
    element_id: Uuid,  // <-- Which element to tombstone
}
```

**TLA+ Equivalent**: `RGA.tla` lines 40-48
```tla
RGAElement == [
  id: ElemIds \union {0},
  value: Values \union {"TOMBSTONE"},
  tombstone: BOOLEAN,
  afterId: ElemIds \union {0},  \* 0 means head
  vc: VectorClock
]
```

#### RGA Structure in Rust
**Location**: List elements are stored with position references in the database

**Key Properties**:
- Each element has immutable UUID (`element_id`)
- Elements reference the element they were inserted after (`after_id`)
- Deletions mark elements as tombstones (don't remove from structure)
- Ordering is reconstructed by following `after_id` references
- Concurrent inserts at same position are ordered by vector clock

**TLA+ Equivalent**: `RGA.tla` lines 161-185
```tla
InsertAfter(node, elemId, value, afterId) ==
  /\ elemId \notin {e.id : e \in nodeList[node]}  \* New ID
  /\ afterId = 0 \/ HasElement(nodeList[node], afterId)  \* Valid reference
  /\ LET vc == Increment(nodeVC[node], node)
         newElem == [
           id |-> elemId,
           value |-> value,
           tombstone |-> FALSE,
           afterId |-> afterId,
           vc |-> vc
         ]
     IN nodeList' = [nodeList EXCEPT ![node] = @ \union {newElem}]
```

**Tombstone Delete**: `RGA.tla` lines 187-209
```tla
DeleteElement(node, elemId) ==
  /\ HasElement(nodeList[node], elemId)
  /\ LET elem == FindElement(nodeList[node], elemId)
     IN nodeList' = [nodeList EXCEPT ![node] =
          (@ \ {elem}) \union {[elem EXCEPT !.tombstone = TRUE]}]
```

---

## Summary Table

| CRDT Type | TLA+ File | Lines | Rust Implementation | Key Line Numbers |
|-----------|-----------|-------|---------------------|------------------|
| **LWW** | LWW.tla | 297 | `raisin-rocksdb/replication/application.rs` | 1460-1499 |
| | | | `raisin-hlc/lib.rs` | 62-68, 225-232 |
| **Add-Wins Set** | AddWinsSet.tla | 358 | `raisin-rocksdb/replication/application.rs` | 2437-2572 |
| | | | `raisin-replication/operation.rs` | 181-199 |
| **Delete-Wins** | DeleteWins.tla | 387 | `raisin-rocksdb/replication/application.rs` | 1501-1559 |
| | | | `raisin-replication/operation.rs` | 217-230 |
| **RGA** | RGA.tla | 406 | `raisin-replication/operation.rs` | 232-250 |

---

## Verification Correspondence

Each TLA+ property maps to a runtime guarantee in the Rust implementation:

### LWW Properties → Runtime Guarantees
- **Determinism** (TLA+) → Same operations always produce same winner (Rust: HLC comparison)
- **Commutativity** (TLA+) → Operation order doesn't matter (Rust: versioned keys)
- **Convergence** (TLA+) → All nodes converge (Rust: `load_latest_node` returns max HLC)

### AddWinsSet Properties → Runtime Guarantees
- **Add-Wins** (TLA+) → Concurrent add/remove → exists (Rust: VC comparison at read)
- **Convergence** (TLA+) → Same ops → same set (Rust: deterministic set computation)

### DeleteWins Properties → Runtime Guarantees
- **Delete-Wins** (TLA+) → Concurrent upsert/delete → deleted (Rust: HLC comparison)
- **No Resurrection** (TLA+) → Once deleted, stays deleted (Rust: tombstone persistence)

### RGA Properties → Runtime Guarantees
- **Causal Order** (TLA+) → Respect happens-before (Rust: VC in element metadata)
- **Tombstone** (TLA+) → Deleted elements remain (Rust: mark as tombstone, don't remove)

---

## How to Use This Mapping

1. **When implementing a CRDT operation in Rust**:
   - Check the corresponding TLA+ spec for the conflict resolution rule
   - Ensure the Rust code follows the same logic

2. **When debugging a conflict resolution issue**:
   - Look at the TLA+ property that failed
   - Trace through the Rust code using the line number mapping

3. **When adding a new CRDT type**:
   - Model it in TLA+ first
   - Verify properties with model checker
   - Implement in Rust following the TLA+ semantics
   - Document the mapping here

---

## References

- TLA+ specs: `/formal/tla/`
- Rust implementation: `/crates/raisin-rocksdb/src/replication/`
- Operations: `/crates/raisin-replication/src/operation.rs`
- HLC: `/crates/raisin-hlc/src/lib.rs`
