---
sidebar_position: 1
---

# Operate RaisinDB

This guide targets operators running the `raisin-server` binary. All switches and endpoints are derived from `crates/raisin-server/src/main.rs` and `crates/raisin-transport-http/src/routes.rs`.

## Server Configuration

`raisin-server` merges three sources (priority: CLI > `RAISIN_CONFIG` file > defaults):

| Flag / Env | Purpose | Default |
|------------|---------|---------|
| `--port`, `RAISIN_PORT` | HTTP port | `8080` |
| `--bind-address`, `RAISIN_BIND_ADDRESS` | Listen address | `127.0.0.1` |
| `--data-dir`, `RAISIN_DATA_DIR` | RocksDB path | `./.data/rocksdb` |
| `--replication-node-id`, `RAISIN_CLUSTER_NODE_ID` | Cluster identity | `None` |
| `--replication-port`, `RAISIN_REPLICATION_PORT` | TCP replication port | `None` |
| `--replication-peers`, `RAISIN_REPLICATION_PEERS` | Comma-separated peers | `[]` |
| `--monitoring-enabled`, `RAISIN_MONITORING_ENABLED` | Emit metrics | `false` |
| `--monitoring-interval-secs`, `RAISIN_MONITORING_INTERVAL_SECS` | Metrics cadence | `30` |
| `--monitoring-port`, `RAISIN_MONITORING_PORT` | Dedicated metrics port | falls back to HTTP |

Load the same keys from `examples/cluster/node1.toml` for reproducible deployments.

## Storage Options

- **RocksDB (`storage-rocksdb` feature)** – production mode with replication, embeddings, and index maintenance. Implements the `Storage` trait and exposes dedicated admin endpoints.
- **In-memory (`raisin-storage-memory`)** – no persistence, useful for tests and demos.
- **Binary storage** – filesystem and S3 backends selected via cargo features (`raisin-binary`).

## Replication

When `replication.enabled = true`:

- `raisin-replication` spawns a TCP server (`crates/raisin-replication/src/tcp_server.rs`).
- HTTP exposes `/api/replication/{tenant}/{repo}/operations` plus batch/apply/vector-clock helpers.
- Use `/api/management/repositories/{tenant}/{repo}/branches/{branch}/compare/{base}` and `/merge` (RocksDB only) for Git-style workflows.

## Authentication & Admin APIs

Enabled under `storage-rocksdb`:

- `/api/raisindb/sys/{tenant}/auth` – obtain tokens.
- `/api/raisindb/sys/{tenant}/auth/change-password` – update admin credentials (protected by middleware).
- `/api/raisindb/sys/{tenant}/admin-users` – manage administrator accounts.

## Index Management

The management routes under `/api/admin/management/database/{tenant}/{repo}` (see `routes.rs`) let you:

- `fulltext/verify|rebuild|optimize|purge|health`
- `vector/verify|rebuild|regenerate|optimize|restore|health`

These handlers call into `raisin-indexer` and `raisin-embeddings` for Tantivy and vector index maintenance.

## Global & Tenant Maintenance

- `/api/admin/management/global/rocksdb/compact|backup|stats`
- `/api/admin/management/tenant/{tenant}/cleanup|stats`

Call these endpoints with admin authentication to keep disk usage under control and monitor per-tenant quotas.

## Monitoring

Enable monitoring in the config to start the background task described near `monitoring_enabled` in `main.rs`. Metrics are emitted via tracing subscribers; wire them into your observability stack (Prometheus, OTLP, etc.).

## Upgrade Playbook

1. **Drain ingress** – stop accepting new write traffic.
2. **Snapshot** – run `/api/admin/management/global/rocksdb/backup`.
3. **Rolling restart** – deploy updated binaries node by node.
4. **Verify vector clocks** – hit `/api/replication/{tenant}/{repo}/vector-clock` to confirm cluster convergence.

Following these steps keeps you aligned with what the code paths guarantee today.
