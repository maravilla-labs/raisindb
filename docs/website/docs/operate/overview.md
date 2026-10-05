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

- `fulltext/verify|rebuild|reconcile|optimize|purge|health|errors`
- `vector/verify|rebuild|regenerate|optimize|restore|health`

These handlers call into `raisin-indexer` and `raisin-embeddings` for Tantivy and vector index maintenance. Each takes an optional `?branch=` (default: the repository's default branch). Which one to run:

- `fulltext/rebuild` after recreating a repository or changing its languages. Changing the default language through `PATCH /api/repositories/{repo}/translation-config` queues this rebuild for every branch by itself. It indexes base content under the repository's `default_language` and `supported_languages`; up to v0.6.45 rebuild and reconcile used `en` for every repository, so rebuild once on v0.6.46 or later.
- `vector/rebuild` re-adds the stored embeddings to the HNSW index, with no embedding provider calls.
- `vector/regenerate` queues re-embedding for stored vectors with the wrong dimensions and, from v0.6.46, for embedding-eligible nodes that have no embedding; `?force=true` re-embeds every stored embedding.

### Compound indexes build themselves

Compound indexes are local to each node and are rebuilt in the background by the
`compound_builds` job, one branch at a time, paced and only with enough free
disk (twice the compound index's size), after the job system starts and again
after a checkpoint is ingested. It builds:

- the built-in folder index (`@__children_by_created_at`, see
  [Indexes](../access/sql/indexes.md)) on every workspace that has not opted out
  (`config.builtin_indexes.children_by_created_at: false`), and drops the entries
  of a workspace that opted out;
- every compound index whose build-state record is from an older format, so no
  manual rebuild is needed after an upgrade.

Until an index is rebuilt on a node, queries there scan and return the same rows
more slowly. The job is visible and can be started like any index repair:
`POST /api/management/{repo}/repairs/compound_builds` fans it out to every node
and `GET /api/management/{repo}/repairs/compound_builds/status` reports each
node's state.

A build that finds nodes with no value for the index's order column (nodes
written before `created_at`/`updated_at` were stamped, up to v0.1.75) does not
run, because the index could not list those nodes. That is an expected state,
not a job failure: the job logs one warning with the node count, saves its
status as `refused_missing_order_values` with the count in
`refused_missing_order_values` (shown by the status endpoint), and is not
retried. The `timestamp_backfill` job below fixes those nodes and asks for the
build again when it finishes.

### Legacy nodes get their timestamps

The `timestamp_backfill` job gives every live node whose newest version has no
`created_at` the time of its first stored revision on that branch (the oldest
one kept, if history cleanup removed the first), and a missing `updated_at` the
time of its newest revision. Nodes that already have both are not touched,
deleted nodes are skipped, and nothing else on the node changes.

Each fixed node is rewritten in place, at the revision of the version that was
read, as the `system` user through the normal write path. It gets no new
revision and the branch head does not move, so history, branch comparison and
merges are unchanged. Any edit made at the same time has a later revision and
wins, on this server and on every replica. If the node changes between the
read and the write, nothing is written and the next run tries again. Every
index is updated, and the change replicates to other nodes like any other
write. No node events are published, so triggers, webhooks and subscriptions
do not fire for this job.

It runs by itself after the job system starts and again after a checkpoint is
ingested, one branch at a time, paced, and resumes where it stopped after a
restart. A branch whose compound index build was refused because of nodes
without timestamps is picked up again at the next start. Each batch first
checks that the disk has room for what it writes. When a branch is done and
nodes were fixed, its compound index builds are requested again. To run
it by hand: `POST /api/management/{repo}/repairs/timestamp_backfill` (body
`{"dry_run": true}` only counts; `"branch"` limits it to one branch) and
`GET /api/management/{repo}/repairs/timestamp_backfill/status`.

| Env | Purpose | Default |
|-----|---------|---------|
| `RAISIN_COMPOUND_FORMAT_REBUILD` | `0`/`false`/`off`/`no` stops the automatic rebuild of older-format compound indexes; they then wait for a manual rebuild (`reindex/start` with `index_types: ["compound"]`) | on |
| `RAISIN_TIMESTAMP_BACKFILL` | `0`/`false`/`off`/`no` stops the automatic `timestamp_backfill` job. Folder listings on workspaces with legacy nodes then keep scanning (correct, slower), and the endpoint above still runs the job by hand | on |

## Deleting a Repository

`DELETE /api/repositories/{repo_id}` (or `raisindb repo delete <repo> --yes`) is irreversible. From v0.6.46 it removes all of the repository's data: every key it owns in every column family (nodes, revisions, branches, tags, translations, types, embeddings, indexes), the registry entry, its jobs including queued full-text and embedding jobs, and the full-text and vector index directories. In a cluster the delete is replicated and each peer removes its own copy. It keeps tenant-wide data (identities, sessions, admin users, tenant AI/auth/embedding configuration), the query-embedding cache, and uploaded binaries in the binary store, whose files can be referenced from more than one place. Up to v0.6.45 the delete removed only the registry entry, so a repository recreated under the same id came back with the old data.

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
