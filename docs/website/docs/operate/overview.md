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

Enabled under `storage-rocksdb`. Two distinct user stores — don't mix them up:

**Admin users** (console/CLI/API operators, tenant-scoped):

- `/api/raisindb/sys/{tenant}/auth` – obtain tokens.
- `/api/raisindb/sys/{tenant}/auth/change-password` – update admin credentials (protected by middleware).
- `/api/raisindb/sys/{tenant}/admin-users` – manage administrator accounts.

**Identities** (application end users, the pluggable auth system):

- `/auth/login`, `/auth/{repo}/login` – identity login, returns a user token.
- `/auth/change-password`, `/auth/{repo}/change-password` – identity changes its
  own password. Tenant and identity come from the token, so it can only act on
  the caller's own account. Clears `must_change_password`.
- `/api/raisindb/sys/{tenant}/identity-users` – manage identities with a
  per-tenant admin JWT.

## Operator (Superadmin) Surface

`/management/admin/*` holds the cross-tenant powers a hosting control plane
needs: tenant provisioning, credential recovery, identity provisioning, and
incident response.

Gated by `Authorization: Bearer $RAISIN_SUPERADMIN_TOKEN`, compared in constant
time. **If `RAISIN_SUPERADMIN_TOKEN` is unset or empty the subtree is not
mounted at all** — callers see 404 rather than 401, so the surface can't be
probed for existence. To enable, set it to a long random string at server start.
Rotation is by restart with a new value; there is no rotation API. Treat it like
a cloud root credential.

- `POST /management/admin/tenants` – provision a tenant + its initial `admin`
  user. `409` if the tenant already has admin users.
- `DELETE /management/admin/tenants/{tenant}` – wipe all data for a tenant.
- `POST /management/admin/reset-password` – reset the `admin` password for the
  tenant named in `x-tenant-id`.
- `POST|GET /management/admin/tenants/{tenant}/identity-users` – provision or
  list application logins for a tenant, without needing that tenant's admin JWT.
  Caller supplies `repos`, `default_roles`, and `must_change_password`; RaisinDB
  applies no policy of its own and sends no email.
- `GET /management/admin/jobs`, `POST .../jobs/purge-all`,
  `POST .../jobs/force-fail-stuck` – cross-tenant job control.
- `GET /management/admin/health`, `/metrics` – server-wide (not per-tenant).
- `POST /management/admin/compact`, `/backup/all` – cross-tenant maintenance.

If you set `must_change_password` when provisioning an identity, make sure the
client fronting it can call `/auth/change-password` — otherwise the user has no
way to clear the flag and is effectively locked out.

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
