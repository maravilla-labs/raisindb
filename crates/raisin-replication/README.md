# raisin-replication

Operation-based CRDT replication system for RaisinDB clustering and offline sync.

## Overview

This crate provides the core primitives for masterless multi-master replication:

- **Vector Clocks**: Track causal dependencies between operations
- **Operations**: Replayable, idempotent mutations
- **CRDT Merge Rules**: Conflict-free convergence algorithms
- **Causal Delivery**: Ensure operations are applied in dependency order
- **Replay Engine**: Apply operations with conflict detection
- **Garbage Collection**: Bounded growth for operation log

## Key Features

| Feature | Description |
|---------|-------------|
| **Masterless** | No single point of failure, any node can accept writes |
| **Causal Consistency** | Operations applied in happens-before order |
| **Eventual Consistency** | All nodes converge to identical state |
| **Conflict-Free** | Deterministic merge rules for concurrent operations |
| **Offline-First** | Operations queue locally and sync later |

## CRDT Merge Rules

Different operation types use different CRDTs:

| Target | CRDT Type | Behavior |
|--------|-----------|----------|
| Properties | Last-Write-Wins | Vector clock + timestamp + node_id tie-breaking |
| Relations | Last-Write-Wins | Most recent add/remove wins |
| Ordered Lists | RGA | Replicated Growable Array with tombstones |
| Moves | Last-Write-Wins | With conflict event emission |
| Deletes | Delete-Wins | Prevents resurrection of deleted entities |

## Quick Start

```rust
use raisin_replication::{VectorClock, Operation, OpType, CrdtMerge};

// Create a vector clock and increment for local operation
let mut vc = VectorClock::new();
vc.increment("node1");

// Create an operation
let op = Operation::new(
    1,                    // op_seq
    "node1".to_string(),  // cluster_node_id
    vc,                   // vector_clock
    "tenant1".to_string(),
    "repo1".to_string(),
    "main".to_string(),
    OpType::SetProperty {
        node_id: "abc123".to_string(),
        property_name: "title".to_string(),
        value: PropertyValue::String("Hello World".to_string()),
    },
    "user@example.com".to_string(),
);

// Merge concurrent operations using CRDT rules
let result = CrdtMerge::merge_operations(vec![op1, op2]);
```

## Cluster Configuration

Configure replication via TOML:

```toml
node_id = "node1"
replication_port = 9001
bind_address = "0.0.0.0"

[[peers]]
node_id = "node2"
host = "10.0.1.2"
port = 9001

[[peers]]
node_id = "node3"
host = "10.0.1.3"
port = 9001
branch_filter = ["main", "develop"]

[sync]
interval_seconds = 60
batch_size = 1000
realtime_push = true
compression = "zstd"

[connection]
heartbeat_interval_seconds = 30
connect_timeout_seconds = 10
max_connections_per_peer = 4
```

## Operation Types

The system supports comprehensive operation types:

### Node Operations
- `CreateNode`, `DeleteNode`, `RenameNode`, `MoveNode`
- `SetProperty`, `DeleteProperty`
- `SetArchetype`, `SetOrderKey`, `SetOwner`
- `PublishNode`, `UnpublishNode`
- `SetTranslation`, `DeleteTranslation`

### Relation Operations
- `AddRelation`, `RemoveRelation`

### List Operations (RGA CRDT)
- `ListInsertAfter`, `ListDelete`

### Schema Operations
- `UpdateNodeType`, `DeleteNodeType`
- `UpdateArchetype`, `DeleteArchetype`
- `UpdateElementType`, `DeleteElementType`

### Admin Operations
- `UpdateWorkspace`, `DeleteWorkspace`
- `UpdateBranch`, `DeleteBranch`
- `CreateTag`, `DeleteTag`
- `UpdateUser`, `DeleteUser`
- `UpdateTenant`, `DeleteTenant`
- `GrantPermission`, `RevokePermission`

### Auth Operations
- `UpsertIdentity`, `DeleteIdentity`
- `CreateSession`, `RevokeSession`
- `RevokeAllIdentitySessions`
- `RotateRefreshToken`

## Project Usage

### RocksDB Storage (raisin-rocksdb)

The replication system integrates with RocksDB for:
- Operation log persistence (`repositories/oplog/`)
- Persistent idempotency tracking
- Checkpoint creation and transfer
- Index replication (Tantivy, HNSW)

```rust
// raisin-rocksdb/src/replication/
storage.put_operations_batch(&ops).await?;
let ops = storage.get_operations_since(tenant, repo, &vc, limit).await?;
```

### Server (raisin-server)

The server uses replication for:
- Cluster formation and peer discovery
- Real-time push on commit
- Periodic pull synchronization
- Catch-up for new/rejoining nodes

### HTTP Transport (raisin-transport-http)

WebSocket handlers for real-time replication:
- `handlers/replication.rs` - HTTP endpoints
- `handlers/replication_ws.rs` - WebSocket streaming

## Components

| Module | Description |
|--------|-------------|
| `operation` | `Operation` struct and `OpType` enum |
| `vector_clock` | `VectorClock` for causality tracking |
| `crdt` | CRDT merge rules (`CrdtMerge`) |
| `causal_delivery` | Buffer for ordering operations |
| `replay` | `ReplayEngine` with idempotency |
| `coordinator` | `ReplicationCoordinator` orchestration |
| `peer_manager` | Connection pool and peer state |
| `catch_up` | Full-state sync for new nodes |
| `streaming` | Reliable file transfer with checksums |
| `tcp_server` | Low-level TCP replication server |
| `tcp_protocol` | MessagePack protocol messages |
| `gc` | Garbage collection strategies |
| `compaction` | Operation log compaction |
| `config` | Cluster configuration |
| `metrics` | Replication metrics collection |

## Metrics

Comprehensive metrics are available:

```rust
use raisin_replication::{metrics_to_json, ReplicationMetrics};

let metrics = coordinator.get_metrics();
let json = metrics_to_json(&metrics);
```

Key metrics:
- `operations_sent/received_total`
- `sync_duration_ms`
- `causal_buffer_size`
- `idempotency_hit_rate`
- `peer_connection_state`
- `replication_lag_operations`

## Formal Verification

The CRDT implementations have TLA+ specifications in `/formal/tla/`:
- `AddWinsSet.tla` - Add-wins set CRDT
- `DeleteWins.tla` - Delete-wins semantics
- `RGA.tla` - Replicated Growable Array
- `CausalDelivery.tla` - Causal ordering

## Testing

```bash
# Unit tests
cargo test -p raisin-replication

# Property-based tests (proptest)
cargo test -p raisin-replication property_tests

# Integration tests
cargo test -p raisin-replication --test integration_test

# Benchmarks
cargo bench -p raisin-replication
```

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
