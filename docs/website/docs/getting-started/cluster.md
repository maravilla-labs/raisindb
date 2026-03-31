---
sidebar_position: 3
---

# Cluster Quickstart

Spin up a multi-node RaisinDB cluster using the same topology that the `raisin-server` binary expects. All knobs documented here are backed by the CLI flags and config loader in `crates/raisin-server/src/main.rs`.

## When to Use a Cluster

- **High availability** – replicate commits across peers using the replication TCP service defined in `raisin-replication`.
- **Geo separation** – dedicate a node per region while RocksDB keeps deterministic ordering.
- **Hot standby** – keep secondary nodes ready for failover or read scaling.

## Configuration Layout

RaisinDB reads a single TOML file that mirrors the `ServerConfigFile` structure. The example cluster configs live under `examples/cluster/*.toml`. Below is `node1.toml`:

```toml title="examples/cluster/node1.toml"
[server]
port = 8081
bind_address = "127.0.0.1"
data_dir = "./data/node1"
initial_admin_password = "Admin1234567!8"

[replication]
enabled = true
node_id = "node1"
port = 9001
bind_address = "127.0.0.1"

[[replication.peers]]
peer_id = "node2"
address = "127.0.0.1"
port = 9002

[[replication.peers]]
peer_id = "node3"
address = "127.0.0.1"
port = 9003
```

The same keys exist for every peer. Only `node_id`, HTTP/replication ports, and data directories differ.

## Bring Up the Cluster

1. **Compile the server**
   ```bash
   cargo build -p raisin-server --features storage-rocksdb,websocket
   ```

2. **Start node1**
   ```bash
   RAISIN_CONFIG=examples/cluster/node1.toml \
   target/debug/raisin-server
   ```

3. **Repeat for node2/node3**
   - Copy `node1.toml` to `node2.toml` / `node3.toml`.
   - Update `server.port`, `replication.port`, and `replication.node_id`.
   - Start a binary per file.

4. **Verify replication**
   - POST a node to node1 via `/api/repository/{repo}/{branch}/head/{ws}/`.
   - Fetch the same path on node2/3. The replication handler in `crates/raisin-rocksdb/src/replication/application.rs` streams the operation log, so successful replication appears immediately.

## Operational Tips

- **Vector/Fulltext indexes** – the `/api/admin/management/database/{tenant}/{repo}/fulltext/*` and `/vector/*` endpoints are cluster-safe. Run rebuild/verify from a single node.
- **Authentication** – RocksDB builds ship with the admin auth middleware enabled. Use `/api/raisindb/sys/{tenant}/auth` to create session tokens before calling admin or replication APIs.
- **Monitoring** – enable the monitoring section in your config to expose periodic metrics (`ServerConfig.monitoring_*` in `raisin-server`).

Use this as the baseline for staging/production rollouts and adapt tenant IDs, repositories, and peer metadata as needed.
