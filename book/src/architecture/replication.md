# CRDT-Based Replication

RaisinDB uses operation-based CRDTs (Conflict-free Replicated Data Types) for masterless multi-master replication. Any node in a cluster can accept writes, and all nodes converge to identical state without coordination.

## Architecture

The replication system is implemented in the `raisin-replication` crate and consists of:

```
┌─────────────────────────────────────────────────────┐
│                  Replication Layer                    │
│                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────┐ │
│  │ Vector Clock  │  │  Operation   │  │   CRDT     │ │
│  │   Tracking    │  │     Log      │  │  Merge     │ │
│  └──────┬───────┘  └──────┬───────┘  └─────┬──────┘ │
│         └──────────────────┼────────────────┘        │
│                            ▼                         │
│              ┌─────────────────────────┐             │
│              │    Replay Engine        │             │
│              └───────────┬─────────────┘             │
│                          ▼                           │
│              ┌─────────────────────────┐             │
│              │   Causal Delivery       │             │
│              └───────────┬─────────────┘             │
│                          ▼                           │
│              ┌─────────────────────────┐             │
│              │  TCP Protocol / Sync    │             │
│              └─────────────────────────┘             │
└─────────────────────────────────────────────────────┘
```

### Key Modules

| Module | Purpose |
|--------|---------|
| `vector_clock` | Track causal dependencies between operations |
| `operation` | Replayable, idempotent mutation definitions |
| `crdt` | Conflict-free merge rules for each data type |
| `replay` | Apply operations in causal order |
| `causal_delivery` | Ensure operations are delivered respecting causality |
| `coordinator` | Manage replication topology and peer connections |
| `peer_manager` | Track active cluster peers |
| `gc` | Garbage collection for bounded operation log growth |
| `compaction` | Compact operation history |
| `streaming` | Stream operations between nodes |
| `tcp_protocol` / `tcp_server` | Network transport layer |

## Vector Clocks

Vector clocks track causal dependencies in the distributed system. Each cluster node maintains a counter, and the vector clock is a map from node ID to counter value:

```rust
use raisin_replication::VectorClock;

let mut vc = VectorClock::new();
vc.increment("node1"); // node1's counter becomes 1
vc.increment("node1"); // node1's counter becomes 2
```

Vector clock comparison yields one of four results:

- **Before** -- this clock happened before the other (causal predecessor)
- **After** -- this clock happened after the other (causal successor)
- **Concurrent** -- neither clock happened before the other (potential conflict)
- **Equal** -- the clocks are identical

## Operations

Every mutation in RaisinDB is represented as an `Operation` -- a self-contained, replayable unit:

```rust
use raisin_replication::{Operation, OpType, VectorClock};
use raisin_models::nodes::properties::PropertyValue;

let mut vc = VectorClock::new();
vc.increment("node1");

let op = Operation::new(
    1,                              // operation sequence number
    "node1".to_string(),            // originating cluster node
    vc,                             // vector clock at time of operation
    "tenant1".to_string(),
    "repo1".to_string(),
    "main".to_string(),
    OpType::SetProperty {
        node_id: "abc123".to_string(),
        property_name: "title".to_string(),
        value: PropertyValue::String("Hello World".to_string()),
    },
    "user@example.com".to_string(), // actor
);
```

Core operation types include (the full `OpType` enum has many more variants for schema, workspace, branch, auth, and admin operations):

| OpType | Description |
|--------|-------------|
| `CreateNode` | Create a new node with properties |
| `DeleteNode` | Delete a node |
| `SetProperty` | Set a property value on a node |
| `DeleteProperty` | Remove a property from a node |
| `RenameNode` | Rename a node |
| `MoveNode` | Move a node to a new parent |
| `AddRelation` | Add a relationship between nodes |
| `RemoveRelation` | Remove a relationship |
| `ListInsertAfter` | Insert into an ordered list (RGA) |
| `ListDelete` | Remove from an ordered list (RGA) |
| `PublishNode` / `UnpublishNode` | Publish or unpublish a node |
| `SetTranslation` / `DeleteTranslation` | Manage translations |
| `ApplyRevision` | Apply a materialized revision (batch) |
| `UpdateNodeType` / `DeleteNodeType` | Schema operations |

## CRDT Merge Rules

Different operation types use different CRDT strategies for conflict resolution:

### Properties: Last-Write-Wins (LWW)

Property updates use LWW with three-level tie-breaking:

1. **Vector clock** -- causal ordering takes priority
2. **Timestamp** -- wall-clock time breaks ties between concurrent operations
3. **Node ID** -- deterministic string comparison as final tiebreaker

```rust
use raisin_replication::CrdtMerge;

// Two concurrent property updates are merged deterministically
let result = CrdtMerge::merge_operations(vec![op1, op2]);
```

### Relations: Last-Write-Wins

Relationship operations use Last-Write-Wins (LWW) CRDT. Relations are identified by a composite key `(source_id, target_id, relation_type)`, and only one relation of a given type can exist between two nodes. The most recent operation by vector clock wins.

### Ordered Lists: RGA

Ordered lists use the Replicated Growable Array (RGA) algorithm with tombstones, allowing concurrent insertions and deletions to merge without conflicts.

### Moves: Last-Write-Wins

Node moves use LWW. When two nodes concurrently move the same node to different parents, the one with the higher vector clock wins, and a conflict event is emitted for observability.

### Deletes: Delete-Wins

Delete operations take priority over concurrent updates to prevent "resurrection" of deleted nodes.

## Conflict Resolution

When concurrent operations conflict, the merge result indicates whether a conflict occurred:

```rust
match CrdtMerge::merge_operations(ops) {
    MergeResult::Winner(op) => {
        // No conflict, single winner
    }
    MergeResult::Conflict { winner, losers, conflict_type } => {
        // Conflict auto-resolved, but recorded for observability
        // conflict_type: ConcurrentPropertyUpdate, ConcurrentMove, etc.
    }
}
```

Conflict types include:
- `ConcurrentPropertyUpdate` -- two nodes updated the same property simultaneously
- `ConcurrentMove` -- two nodes moved the same node to different parents
- `ConcurrentSchemaUpdate` -- concurrent schema changes
- `DeleteWinsOverUpdate` -- a delete concurrent with an update

## Causal Delivery

The causal delivery module ensures operations are applied in happens-before order. Operations are buffered until all their causal dependencies have been applied, preventing out-of-order execution that could lead to inconsistencies.

## Garbage Collection

The operation log grows with every mutation. The garbage collection system provides bounded growth by:

- Compacting old operations that all nodes have acknowledged
- Maintaining a configurable retention window
- Preserving operations needed for catch-up by lagging nodes

## Cluster Setup

A multi-node cluster uses the replication system automatically. See the [Quick Start](../getting-started/quickstart.md) guide for cluster configuration, and use the provided script to start a test cluster:

```bash
./scripts/start-cluster.sh
```

Each node is configured with its peers in the TOML configuration file. Nodes discover each other and begin streaming operations automatically.
