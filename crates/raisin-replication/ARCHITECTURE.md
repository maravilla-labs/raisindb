# Architecture

## Design Philosophy

raisin-replication implements a masterless multi-master replication system using operation-based CRDTs. Core principles:

1. **Eventual Consistency**: All nodes converge to the same state
2. **Conflict-Free**: Deterministic merge rules eliminate conflicts
3. **Causal Ordering**: Operations respect happens-before relationships
4. **Idempotency**: Operations can be safely re-applied
5. **Partition Tolerance**: System continues during network splits

## Core Data Structures

### Vector Clock

Tracks causal dependencies between operations:

```
┌─────────────────────────────────────────────────────────┐
│                     VectorClock                          │
│                                                          │
│  clock: HashMap<String, u64>                            │
│  { "node1": 5, "node2": 3, "node3": 7 }                │
│                                                          │
│  Operations:                                             │
│  - increment(node_id) → update local counter            │
│  - merge(other) → take max for each node                │
│  - compare(other) → Before | After | Concurrent | Equal │
│  - distance(other) → replication lag                    │
└─────────────────────────────────────────────────────────┘
```

### Operation

The fundamental unit of replication:

```
┌─────────────────────────────────────────────────────────┐
│                      Operation                           │
│                                                          │
│  op_id: Uuid              ← Unique identifier           │
│  op_seq: u64              ← Per-node sequence number    │
│  cluster_node_id: String  ← Origin node                 │
│  timestamp_ms: u64        ← Wall clock (tie-breaking)   │
│  vector_clock: VectorClock ← Causal dependencies        │
│  tenant_id: String        ← Multi-tenant isolation      │
│  repo_id: String          ← Repository scope            │
│  branch: String           ← Branch scope                │
│  op_type: OpType          ← The actual mutation         │
│  revision: Option<HLC>    ← Hybrid logical clock        │
│  actor: String            ← Who performed the action    │
│  acknowledged_by: HashSet ← For GC                      │
└─────────────────────────────────────────────────────────┘
```

### OpType Enum

All possible mutations in the system:

```
OpType
├── Node Operations
│   ├── CreateNode { node_id, name, node_type, properties, ... }
│   ├── DeleteNode { node_id }
│   ├── SetProperty { node_id, property_name, value }
│   ├── DeleteProperty { node_id, property_name }
│   ├── RenameNode { node_id, old_name, new_name }
│   ├── MoveNode { node_id, old_parent_id, new_parent_id, position }
│   └── ...
├── Relation Operations
│   ├── AddRelation { source_id, target_id, relation_type, ... }
│   └── RemoveRelation { source_id, target_id, relation_type, ... }
├── List Operations (RGA)
│   ├── ListInsertAfter { node_id, list_property, after_id, value, element_id }
│   └── ListDelete { node_id, list_property, element_id }
├── Schema Operations
│   ├── UpdateNodeType { node_type_id, node_type }
│   └── ...
└── Admin Operations
    ├── UpdateWorkspace, UpdateBranch, CreateTag, ...
    └── UpsertIdentity, CreateSession, ...
```

## CRDT Merge Rules

### Last-Write-Wins (LWW)

Used for properties, relations, moves, and schema operations:

```
┌─────────────────────────────────────────────────────────┐
│                  LWW Tie-Breaking                        │
│                                                          │
│  1. Vector Clock (causal ordering)                      │
│     - If A happens-before B → B wins                    │
│     - If concurrent → continue to step 2                │
│                                                          │
│  2. Timestamp (wall clock)                              │
│     - Higher timestamp wins                             │
│     - If equal → continue to step 3                     │
│                                                          │
│  3. Node ID (deterministic)                             │
│     - Lexicographically higher node_id wins             │
│     - Ensures all nodes pick same winner                │
└─────────────────────────────────────────────────────────┘
```

### Delete-Wins

Used for node deletions:

```
Delete Operation + Concurrent Update
         │
         ▼
┌─────────────────────────────────────────────────────────┐
│  Delete always wins over concurrent updates to prevent  │
│  "resurrection" of deleted entities.                    │
│                                                          │
│  Node A: Delete node "foo"     VC: {A: 5}              │
│  Node B: Update node "foo"     VC: {B: 3}              │
│                                                          │
│  → Concurrent (neither VC dominates)                    │
│  → Delete wins                                          │
│  → Node "foo" is deleted, update discarded              │
│  → Conflict event emitted for logging                   │
└─────────────────────────────────────────────────────────┘
```

### RGA (Replicated Growable Array)

Used for ordered lists:

```
┌─────────────────────────────────────────────────────────┐
│                    RGA Structure                         │
│                                                          │
│  Each element has:                                       │
│  - element_id: Uuid (immutable, unique)                 │
│  - after_id: Option<Uuid> (insertion point)             │
│  - value: PropertyValue                                  │
│  - vector_clock: VectorClock                            │
│  - tombstone: bool (soft delete)                        │
│                                                          │
│  Insertion Order:                                        │
│  1. Insert after specified element                      │
│  2. Concurrent inserts at same position → VC ordering   │
│  3. Deleted elements become tombstones                  │
│                                                          │
│  Example:                                                │
│  [A] → [B] → [C]                                        │
│  Insert X after A: [A] → [X] → [B] → [C]               │
│  Concurrent insert Y after A: resolved by VC           │
└─────────────────────────────────────────────────────────┘
```

## Causal Delivery

Ensures operations are applied only when dependencies are satisfied:

```
┌─────────────────────────────────────────────────────────┐
│                CausalDeliveryBuffer                      │
│                                                          │
│  Problem:                                                │
│  Node 1: CreateNode(foo) VC:{1:1} → SetProp(foo) VC:{1:2}│
│  Node 2 receives SetProp before CreateNode              │
│  Without causal delivery: SetProp FAILS (no node)       │
│                                                          │
│  Solution:                                               │
│  ┌─────────────────────────────────────────────┐        │
│  │          Causal Delivery Buffer              │        │
│  │                                              │        │
│  │  local_vc: {1:0, 2:0}  ← What we've applied │        │
│  │                                              │        │
│  │  Receive SetProp VC:{1:2}                   │        │
│  │  → Depends on {1:1} (not satisfied)         │        │
│  │  → Buffer operation                          │        │
│  │                                              │        │
│  │  Receive CreateNode VC:{1:1}                │        │
│  │  → All dependencies satisfied               │        │
│  │  → Apply immediately                         │        │
│  │  → Update local_vc to {1:1}                 │        │
│  │  → Check buffer → SetProp now satisfiable   │        │
│  │  → Apply SetProp                             │        │
│  └─────────────────────────────────────────────┘        │
└─────────────────────────────────────────────────────────┘
```

## Replay Engine

Applies operations with idempotency and conflict detection:

```
┌─────────────────────────────────────────────────────────┐
│                    ReplayEngine                          │
│                                                          │
│  1. Check idempotency tracker                           │
│     - If already applied → skip                         │
│                                                          │
│  2. Group by target entity                              │
│     - Operations affecting same node                    │
│                                                          │
│  3. Sort by causal order                                │
│     - Vector clock comparison                           │
│                                                          │
│  4. Apply CRDT merge rules                              │
│     - Determine winner(s) for concurrent ops            │
│                                                          │
│  5. Execute winning operation                           │
│     - Apply to storage                                  │
│                                                          │
│  6. Mark as applied                                     │
│     - Update idempotency tracker                        │
│                                                          │
│  7. Emit conflict events (if any)                       │
│     - For monitoring and debugging                      │
└─────────────────────────────────────────────────────────┘
```

## Replication Coordinator

Orchestrates the entire replication process:

```
┌─────────────────────────────────────────────────────────┐
│              ReplicationCoordinator                      │
│                                                          │
│  ┌─────────────────────────────────────────────────┐    │
│  │              Periodic Sync Loop                  │    │
│  │                                                  │    │
│  │  Every sync_interval:                           │    │
│  │  1. Get local vector clock                      │    │
│  │  2. For each peer:                              │    │
│  │     - Request operations since local VC         │    │
│  │     - Receive batch of operations               │    │
│  │     - Pass to CausalDeliveryBuffer              │    │
│  │     - Deliverable ops → ReplayEngine            │    │
│  │  3. Update local vector clock                   │    │
│  └─────────────────────────────────────────────────┘    │
│                                                          │
│  ┌─────────────────────────────────────────────────┐    │
│  │              Real-time Push                      │    │
│  │                                                  │    │
│  │  On local commit:                               │    │
│  │  1. Create Operation with new VC                │    │
│  │  2. Persist to local oplog                      │    │
│  │  3. Push to all connected peers                 │    │
│  │  4. Await acknowledgments (optional)            │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

## Catch-Up Protocol

For new or rejoining nodes that are far behind:

```
┌─────────────────────────────────────────────────────────┐
│                  Catch-Up Flow                           │
│                                                          │
│  1. New node connects to cluster                        │
│  2. Detects significant lag (operations behind)         │
│  3. Initiates catch-up:                                 │
│                                                          │
│     ┌─────────────────────────────────────────────┐     │
│     │         Checkpoint Transfer                  │     │
│     │                                              │     │
│     │  a. Request checkpoint from peer             │     │
│     │  b. Peer creates atomic snapshot             │     │
│     │  c. Stream SST files with CRC32 checksums   │     │
│     │  d. Verify and rebuild local storage         │     │
│     └─────────────────────────────────────────────┘     │
│                                                          │
│     ┌─────────────────────────────────────────────┐     │
│     │         Index Transfer                       │     │
│     │                                              │     │
│     │  a. Request Tantivy fulltext indexes        │     │
│     │  b. Request HNSW vector indexes             │     │
│     │  c. Stream with checksums and verification   │     │
│     └─────────────────────────────────────────────┘     │
│                                                          │
│  4. Switch to normal sync mode                          │
└─────────────────────────────────────────────────────────┘
```

## TCP Protocol

MessagePack-based protocol for peer communication:

```
┌─────────────────────────────────────────────────────────┐
│               ReplicationMessage Enum                    │
│                                                          │
│  Handshake:                                              │
│  - Hello { node_id, protocol_version }                  │
│  - HelloAck { node_id }                                 │
│                                                          │
│  Sync:                                                   │
│  - SyncRequest { tenant_id, repo_id, since_vc }        │
│  - SyncResponse { operations, has_more }                │
│  - PushOperations { operations }                        │
│  - Acknowledge { op_ids }                               │
│                                                          │
│  Status:                                                 │
│  - StatusRequest                                        │
│  - StatusResponse { log_index, vector_clock, stats }    │
│                                                          │
│  Catch-up:                                               │
│  - CheckpointRequest { snapshot_id }                    │
│  - CheckpointMetadata { files: Vec<SstFileInfo> }       │
│  - FileChunk { data, crc32 }                           │
│  - TantivyIndexRequest { tenant, repo, branch }        │
│  - HnswIndexRequest { tenant, repo, branch }           │
└─────────────────────────────────────────────────────────┘
```

## Garbage Collection

Prevents unbounded operation log growth:

```
┌─────────────────────────────────────────────────────────┐
│                  GC Strategy                             │
│                                                          │
│  An operation can be garbage collected when:            │
│                                                          │
│  1. All peers have acknowledged receiving it            │
│     (tracked in operation.acknowledged_by)              │
│                                                          │
│  2. OR operation is older than retention period         │
│     (configurable, e.g., 7 days)                        │
│                                                          │
│  3. AND no uncommitted transaction depends on it        │
│                                                          │
│  ┌─────────────────────────────────────────────────┐    │
│  │              Watermark Tracking                  │    │
│  │                                                  │    │
│  │  Each peer reports its highest received op_seq  │    │
│  │  Global minimum watermark = safe GC point       │    │
│  │  Operations below watermark can be compacted    │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

## Compaction

Reduces operation log size:

```
┌─────────────────────────────────────────────────────────┐
│              Operation Log Compaction                    │
│                                                          │
│  1. Identify superseded operations                      │
│     - SetProperty(foo.title, "A")                       │
│     - SetProperty(foo.title, "B") ← supersedes above   │
│     → Keep only the latest                              │
│                                                          │
│  2. Merge tombstones with creates                       │
│     - CreateNode(foo) + DeleteNode(foo)                │
│     → Can be fully removed if no dependencies           │
│                                                          │
│  3. Snapshot nodes periodically                         │
│     - Replace sequence of property sets                 │
│     → Single UpsertNodeSnapshot operation               │
└─────────────────────────────────────────────────────────┘
```

## Module Dependencies

```
raisin-replication/
├── operation.rs          ← Core Operation type
├── vector_clock.rs       ← Causality tracking
├── crdt.rs              ← Merge rules
├── causal_delivery.rs   ← Ordering buffer
├── replay.rs            ← Apply engine
├── coordinator.rs       ← Orchestration
├── peer_manager.rs      ← Connection pool
├── catch_up.rs          ← Full-state sync
├── streaming.rs         ← File transfer
├── tcp_server.rs        ← Server socket
├── tcp_protocol.rs      ← Message types
├── tcp_helpers.rs       ← I/O utilities
├── gc.rs                ← Garbage collection
├── compaction.rs        ← Log compaction
├── config.rs            ← Configuration
├── metrics.rs           ← Observability
├── metrics_reporter.rs  ← JSON export
├── priority.rs          ← Operation ordering
├── conflict_resolution.rs ← Conflict handling
├── operation_decomposer.rs ← Batch splitting
└── value_conversion.rs  ← JSON ↔ MessagePack
```

## Thread Safety

All components are designed for concurrent access:

- `VectorClock`: Clone + Send + Sync
- `Operation`: Clone + Send + Sync
- `CausalDeliveryBuffer`: Internal locking
- `ReplayEngine`: Stateless, takes `&mut IdempotencyTracker`
- `ReplicationCoordinator`: Arc<RwLock<...>> internally
- `PeerManager`: Thread-safe connection pool
