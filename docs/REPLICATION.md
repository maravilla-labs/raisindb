# RaisinDB Cluster Replication Guide

This guide explains how to set up and use RaisinDB's TCP-based database-to-database replication system.

## Overview

RaisinDB uses a **masterless multi-master replication** architecture based on **operation-based CRDTs** (Conflict-free Replicated Data Types). This means:

- ✅ **No single point of failure** - Any node can accept writes
- ✅ **Automatic conflict resolution** - Deterministic merge rules ensure eventual consistency
- ✅ **Real-time & periodic sync** - Push changes immediately and pull missing operations periodically
- ✅ **Causal consistency** - Operations are applied in happens-before order
- ✅ **Offline-capable** - Operations queue locally and sync when connectivity is restored

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ Node 1 (10.0.1.1:9001)                                      │
│                                                              │
│  Application → RocksDB → OpLog → ReplicationCoordinator     │
│                              ↓            ↓           ↓      │
│                           OpLog      TCP Client   TCP Server │
└──────────────────────────────────────┬────────┬─────────────┘
                                       │        │
                    TCP Port 9001      │        │
                    MessagePack Binary │        │
                                       │        │
┌──────────────────────────────────────┴────────┴─────────────┐
│ Node 2 (10.0.1.2:9001)                                      │
│                                                              │
│  Application → RocksDB → OpLog → ReplicationCoordinator     │
│                              ↓            ↓           ↓      │
│                           OpLog      TCP Client   TCP Server │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

### 1. Create Cluster Configuration

Create `config/cluster.toml` for each node:

**Node 1:**
```toml
[cluster]
node_id = "node1"
replication_port = 9001
bind_address = "0.0.0.0"

[[peers]]
node_id = "node2"
host = "10.0.1.2"
port = 9001
enabled = true

[sync]
interval_seconds = 30
batch_size = 1000
realtime_push = true
```

**Node 2:**
```toml
[cluster]
node_id = "node2"
replication_port = 9001
bind_address = "0.0.0.0"

[[peers]]
node_id = "node1"
host = "10.0.1.1"
port = 9001
enabled = true

[sync]
interval_seconds = 30
batch_size = 1000
realtime_push = true
```

### 2. Enable Replication in Code

```rust
use raisin_rocksdb::{RocksDBStorage, replication::start_replication};
use raisin_replication::ClusterConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize RocksDB storage with replication enabled
    let mut config = raisin_rocksdb::Config::default();
    config.replication_enabled = true;  // Enable operation capture

    let db = Arc::new(RocksDBStorage::new_with_config("/data/rocksdb", config)?);

    // 2. Load cluster configuration
    let cluster_config = ClusterConfig::from_toml_file("config/cluster.toml")?;

    // 3. Start replication coordinator
    let coordinator = start_replication(db.clone(), cluster_config).await?;

    println!("Replication started - Node ID: {}", coordinator.get_sync_stats().await.cluster_node_id);

    // 4. Use the database normally - all writes are automatically replicated
    // ... your application code ...

    Ok(())
}
```

### 3. Monitor Replication Status

```rust
// Get sync statistics
let stats = coordinator.get_sync_stats().await;
println!("Cluster Node: {}", stats.cluster_node_id);
println!("Total Peers: {}", stats.total_peers);
println!("Connected: {}", stats.connected_peers);
println!("Disconnected: {}", stats.disconnected_peers);
```

## How It Works

### Operation Capture

Every write operation (create, update, delete) is captured to the operation log:

```rust
// Transaction writes (automatic capture)
let tx = storage.begin().await?;
tx.nodes().create("tenant1", "repo1", "main", "workspace1", node).await?;
tx.commit().await?;  // ← Operations captured and pushed to peers

// Non-transaction writes (automatic capture)
storage.nodes().add("tenant1", "repo1", "main", "workspace1", node).await?;
// ← Operation captured and (if no queue) immediately available for sync
```

### Replication Modes

RaisinDB supports two replication modes that work together:

#### 1. Real-Time Push (Immediate)
- Triggered after transaction commit
- Operations pushed to all connected peers via TCP
- Low-latency replication (milliseconds)
- Enabled by default (`realtime_push = true`)

```
Node A writes → Commit → Push to peers → Node B receives & applies
Time: ~10-50ms (depending on network latency)
```

#### 2. Periodic Pull (Every N seconds)
- Each node periodically pulls missing operations from peers
- Resilient to temporary network failures
- Ensures eventual consistency even if push fails
- Configurable interval (default: 30 seconds)

```
Node B → Query own vector clock → Pull from Node A → Apply missing ops
Time: Up to interval_seconds (default 30s)
```

### Conflict Resolution

RaisinDB uses **operation-based CRDTs** with deterministic merge rules:

| Operation Type | Conflict Resolution Strategy |
|----------------|------------------------------|
| **Properties** | Last-Write-Wins (LWW) with vector clock + timestamp + node_id tie-breaking |
| **Relations** | Add-Wins Set CRDT (additions always win over deletions) |
| **Deletes** | Delete-Wins (prevents resurrection of deleted nodes) |
| **Moves** | Last-Write-Wins with conflict event emission |

**Example Concurrent Writes:**

```
Time: T1
Node A: SET document.title = "Hello"    {vector_clock: {A:1}}
Node B: SET document.title = "World"    {vector_clock: {B:1}}

After Sync (both nodes converge):
→ document.title = "World"  (tie-breaker: node_id "B" > "A")
```

## Configuration Options

### Cluster Settings

```toml
[cluster]
node_id = "unique-node-identifier"    # Required, must be unique per node
replication_port = 9001                # TCP port for replication (default: 9001)
bind_address = "0.0.0.0"              # Network interface (0.0.0.0 = all interfaces)
```

### Peer Configuration

```toml
[[peers]]
node_id = "peer-node-id"              # Unique identifier of peer node
host = "10.0.1.2"                     # IP address or hostname
port = 9001                           # Replication port on peer
enabled = true                        # Enable/disable this peer
priority = 1                          # Priority for sync order (higher = first)
branch_filter = ["main", "prod"]      # Optional: only sync specific branches
```

### Sync Settings

```toml
[sync]
interval_seconds = 30                 # Pull sync interval (0 = disabled)
batch_size = 1000                     # Max operations per sync batch
realtime_push = true                  # Enable immediate push after writes
```

### Connection Settings

```toml
[connection]
connect_timeout_seconds = 10          # TCP connection timeout
read_timeout_seconds = 30             # Message read timeout
write_timeout_seconds = 30            # Message write timeout
heartbeat_interval_seconds = 30       # Heartbeat ping interval
```

### Retry Settings

```toml
[sync.retry]
max_attempts = 5                      # Max retry attempts before giving up
base_delay_ms = 1000                  # Initial retry delay (exponential backoff)
max_backoff_ms = 60000               # Max retry delay cap
jitter_factor = 0.1                   # Random jitter (0.0-1.0)
```

## Network Requirements

### Ports
- **9001/TCP** - Replication protocol (configurable via `replication_port`)
- Ensure firewall allows bidirectional traffic between cluster nodes

### Bandwidth Estimation
- **Typical operation**: ~200-500 bytes (MessagePack binary)
- **Batch of 1000 operations**: ~200-500 KB
- **Recommendation**: 1 Mbps minimum, 10 Mbps for high-throughput clusters

### Latency Requirements
- **LAN deployment**: < 10ms RTT recommended
- **WAN deployment**: < 100ms RTT (higher latency increases sync lag)
- **Real-time push**: Best with < 50ms latency

## Operational Scenarios

### Adding a New Node to the Cluster

1. **Configure the new node** with cluster configuration listing existing peers
2. **Start the new node** - it will connect to peers and begin syncing
3. **Update existing nodes** - add the new peer to their cluster.toml
4. **Restart or reload** existing nodes to connect to the new peer

**Note:** Currently requires restart. Dynamic peer addition coming in future release.

### Handling Network Partitions

RaisinDB is **partition-tolerant**:

1. **During partition**: Each partition accepts writes independently
2. **After partition heals**: Periodic pull sync kicks in automatically
3. **Conflict resolution**: CRDT merge rules ensure deterministic convergence
4. **No manual intervention required**

Example:
```
T0: [Node A] ←→ [Node B] ←→ [Node C]    (all connected)
T1: [Node A]     [Node B] ←→ [Node C]    (A partitioned)
T2: Write to A   Write to B              (concurrent writes)
T3: [Node A] ←→ [Node B] ←→ [Node C]    (partition heals)
T4: Automatic sync and conflict resolution
```

### Monitoring and Troubleshooting

#### Check Replication Status

```rust
let stats = coordinator.get_sync_stats().await;
if stats.disconnected_peers > 0 {
    println!("Warning: {} peers disconnected", stats.disconnected_peers);
}
```

#### View Operation Log

```rust
use raisin_rocksdb::repositories::OpLogRepository;

let oplog = OpLogRepository::new(db.db().clone());
let ops = oplog.get_all_operations("tenant1", "repo1")?;
println!("Total operations: {}", ops.len());
```

#### Common Issues

| Issue | Cause | Solution |
|-------|-------|----------|
| "Peer not connected" | Network issue, firewall | Check network connectivity, open port 9001 |
| "Protocol version mismatch" | Different RaisinDB versions | Upgrade all nodes to same version |
| "Operation batch timeout" | Large batch, slow network | Reduce `batch_size` in config |
| "High replication lag" | Network latency, overload | Reduce `interval_seconds`, increase bandwidth |

## Performance Tuning

### For Low-Latency Clusters (LAN)
```toml
[sync]
interval_seconds = 10       # More frequent sync
batch_size = 2000          # Larger batches for efficiency
realtime_push = true       # Immediate propagation

[connection]
heartbeat_interval_seconds = 15
```

### For High-Latency Clusters (WAN)
```toml
[sync]
interval_seconds = 60       # Less frequent sync to reduce overhead
batch_size = 500           # Smaller batches for timeout safety
realtime_push = false      # Rely on pull sync (more resilient)

[connection]
read_timeout_seconds = 60
write_timeout_seconds = 60
heartbeat_interval_seconds = 60
```

### For High-Throughput Clusters
```toml
[sync]
batch_size = 5000          # Large batches for efficiency
realtime_push = true       # Immediate push reduces lag

# Enable async operation queue (set in code)
config.operation_queue_capacity = 100000
```

## Security Considerations

### Current Implementation
- ✅ Cluster nodes authenticate via node_id during handshake
- ⚠️ No encryption - traffic is unencrypted MessagePack binary
- ⚠️ No authentication beyond node_id verification

### Recommended for Production
1. **Use VPN or private network** for cluster communication
2. **Firewall rules** - restrict port 9001 to cluster nodes only
3. **Network isolation** - keep replication traffic on dedicated VLAN

### Future Enhancements (Planned)
- 🔒 Mutual TLS for encryption and authentication
- 🔑 Token-based authentication
- 🔐 Optional ZSTD compression for bandwidth efficiency

## Example: 3-Node Cluster Setup

```toml
# Node 1 (10.0.1.1) - config/cluster.toml
[cluster]
node_id = "node1"
bind_address = "0.0.0.0"
replication_port = 9001

[[peers]]
node_id = "node2"
host = "10.0.1.2"
port = 9001
enabled = true

[[peers]]
node_id = "node3"
host = "10.0.1.3"
port = 9001
enabled = true

[sync]
interval_seconds = 30
batch_size = 1000
realtime_push = true
```

Repeat for Node 2 and Node 3, updating `node_id` and `host` accordingly.

**Topology:**
```
    Node 1 ←→ Node 2
       ↖   ↗
        Node 3
```

Each node connects to every other node (full mesh).

## Best Practices

1. **Node IDs**: Use descriptive, stable identifiers (e.g., hostname, datacenter+rack)
2. **Clock Sync**: Ensure NTP is configured on all nodes (for accurate timestamps)
3. **Monitoring**: Set up alerts for `disconnected_peers > 0`
4. **Backups**: Regular backups even with replication (protects against data corruption)
5. **Testing**: Test network partition scenarios in staging before production
6. **Gradual Rollout**: Add nodes one at a time, verify sync before adding next

## Limitations (Current Version)

- ❌ No dynamic peer discovery - requires explicit configuration
- ❌ No automatic peer addition without restart
- ❌ No built-in encryption (use VPN/network isolation)
- ❌ No bandwidth throttling (uses all available bandwidth)
- ❌ No compaction of operation log (grows unbounded - manual GC required)

## Next Steps

- See [config/cluster.example.toml](../config/cluster.example.toml) for full configuration reference
- Check logs with `RUST_LOG=raisin_replication=debug` for detailed replication events
- Use `cargo test --package raisin-replication` to run replication tests

---

**Questions or Issues?**
Report at https://github.com/maravilla-labs/raisindb/issues
