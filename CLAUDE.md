# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
# Build the server (production)
cargo build --release --package raisin-server --features "storage-rocksdb,websocket,pgwire"

# Build all crates
cargo build --workspace

# Run the server with pgwire enabled
RUST_LOG=info ./target/release/raisin-server --config examples/cluster/node1.toml --pgwire-enabled true

# Run all workspace tests
cargo test --workspace

# Run a single test file
cargo test --package raisin-server --test cluster_social_feed_test -- --ignored --nocapture

# Run a specific test
cargo test --package raisin-server --test cluster_social_feed_test test_add_post_node1 -- --ignored --nocapture

# Run benchmarks
cargo bench -p raisin-rocksdb

# Quality checks
cargo fmt --workspace
cargo clippy --workspace
```

## Project Architecture

RaisinDB is a multi-tenant content database with CRDT-based replication. Key layers:

```
Transport (HTTP/WS/PGWire)  →  Core Business Logic  →  Storage Abstraction  →  RocksDB/Memory
```

### Core Crates

| Crate | Purpose |
|-------|---------|
| `raisin-core` | Business logic, NodeService, WorkspaceService, validation |
| `raisin-models` | Data types (Node, NodeType, PropertyValue) |
| `raisin-storage` | Storage traits and ScopedStorage wrapper |
| `raisin-rocksdb` | RocksDB implementation with 40+ column families |
| `raisin-sql` | SQL parser, analyzer, logical planner |
| `raisin-sql-execution` | Physical plan execution, Cypher/PGQ support |
| `raisin-replication` | CRDT-based multi-master replication |
| `raisin-hlc` | Hybrid Logical Clock for versioning |

### Transport Crates

| Crate | Purpose |
|-------|---------|
| `raisin-transport-http` | Axum-based REST API |
| `raisin-transport-ws` | WebSocket real-time events |
| `raisin-transport-pgwire` | PostgreSQL wire protocol (connect via `psql`) |

### Key Patterns

**Multi-tenancy**: All data keys are prefixed with `{tenant}\0{repo}\0{branch}\0{workspace}\0...`. Use `ScopedStorage` for automatic isolation.

**Storage Key Encoding**: Uses descending revisions (`~revision = u64::MAX - revision`) for efficient "latest" queries via prefix scans.

**Job Queue**: Always use the unified job queue via `JobRegistry.register_job()` + `JobDataStore.put()` for async tasks.

## Feature Flags

Server features in `raisin-server/Cargo.toml`:
- `storage-rocksdb` (default) - RocksDB backend
- `store-memory` - In-memory storage for testing
- `websocket` (default) - WebSocket transport
- `pgwire` (default) - PostgreSQL wire protocol
- `ai` (default) - AI/ML features
- `fs` (default) / `s3` - Binary storage backends

## Testing

- **Unit tests**: In-crate `#[cfg(test)]` modules
- **Integration tests**: `crates/*/tests/` directories
- **Cluster tests**: `crates/raisin-server/tests/cluster_*.rs` (marked `#[ignore]`, run with `--ignored`)

Start a 3-node test cluster: `./scripts/start-cluster.sh`

## RaisinDB SQL Syntax

### JSON Property Queries

Query JSON properties with the `->>` operator. **Prefer the `::String` key-cast
form** — it evaluates as a verbatim row-level filter, so it is always correct,
including when combined with `path =` / `node_type =` and on workspaces that have
compound indexes:

```sql
-- ✅ Recommended: cast the key. Verbatim filter, always correct.
SELECT * FROM 'workspace' WHERE properties->>'user_id'::String = $1

-- ✅ Property predicate combined with a path/id equality:
UPDATE 'workspace' SET properties = $1::jsonb
  WHERE path = $2 AND properties->>'seq'::String = $3

-- ✅ Number-valued properties: ->> yields text, so compare against a string.
--    (e.g. seq stored as JSON 0 → compare against '0', not 0)
SELECT * FROM 'workspace' WHERE properties->>'seq'::String = '0'
```

Notes:
- `->>` always yields **text**; compare against a string literal even for
  number/bool-valued properties (`properties->>'seq'::String = '0'`).
- The bare form (`properties->>'k' = v`, no cast) is canonicalized and may be
  routed to the `property_index` or a **compound index**. It now matches
  correctly when combined with another predicate (fixed `JsonPropertyEq::to_expr`,
  which previously rebuilt it as `@>` and dropped the key — that was the old
  "no cast returns empty results" symptom). However, if a matching compound index
  is unbuilt/stale it can still return zero rows — so when in doubt, use the cast
  form.

## Node Revision History & Authorship

**Authorship is stamped at the transaction layer.** `created_by` / `updated_by`
(and `updated_at`) are filled in the two low-level write functions every path
funnels through: `add_node.rs` (the optimized CREATE path) and `put_node.rs`
(create-or-update), both under `raisin-rocksdb/.../create/core/`. They resolve
the actor from the transaction's auth context (`actor_id()`) → raw actor →
`"anonymous"`. `put_node` preserves the original `created_by`/`created_at` on
update. Don't re-stamp these in higher layers — and if you add a new low-level
write path, stamp it there too.

**Revision history (git-style "file history")** lists a node's MVCC revisions,
newest first, via `NodeService::history(id, limit)` →
`NodeRepository::get_node_history` (reuses `get_history`). Entries are lightweight
(`NodeRevisionEntry { revision, updated_at, updated_by, deleted }`); use the
`revision` with the `rev/{revision}` time-travel reads to fetch a full snapshot.
Always available regardless of `auditable` — it's structural, not opt-in.
Surfaces:
- **HTTP**: `GET /api/history/{repo}/{branch}/{ws}/by-id/{id}` and `/{*node_path}` (`?limit=`)
- **WS**: `node_history` request (`{ node_id | path, limit }`)
- **JS SDK**: `ws.nodes().history(id, { limit })` / `historyByPath(path, …)`
- **Functions**: `raisin.nodes.history(workspace, id, limit?)`

**The `auditable` NodeType flag gates the audit log** (not history). Audit-log
entries are written only when an audit sink is configured AND the node's NodeType
has `auditable = true`, enforced in `NodeService::audit_write` (`node_service/core.rs`).
Non-auditable types produce no audit-log entries but still have full MVCC history.
The audit sink (`RepoAuditAdapter` → `InMemoryAuditRepo`) is built once in `main.rs`
and shared by both transports: HTTP and WS each wire it via `NodeService::with_audit`,
and the same `audit_repo` backs the read APIs. Query audit logs via:
- **HTTP**: `GET /api/audit/{repo}/{branch}/{ws}/by-id/{id}` and `/{*node_path}`
- **WS**: `audit_query` request (`{ node_id | path }`)
The adapter records `node.updated_by` as the log's `user_id`, so audit authorship is
reliable now that authorship is stamped at the transaction layer (above).

## Atomic Locks & Inventory (`raisin-locks`)

Backend-pluggable acquire / tie-breaker primitive for ticket-sale-style workloads.
Opt-in via the `[locks]` config section (`enabled`, `backend = "inprocess" | "redis"`).
Build with `--features locks-redis` (server) to enable the Redis backend.

- **Lease-lock**: `try_acquire(key, owner, ttl)` returns a monotonic **fencing
  token** or `None` (held). Pass the token into the guarded write and reject
  stale tokens to prevent a paused holder from clobbering newer state.
- **Counting reservation**: `claim(pool, n, capacity)` atomically decrements a
  pool, never going below zero — the "N seats left" primitive.
- **`inprocess` is single-node only.** Multi-node clusters MUST use `redis` or
  they will oversell; the server logs a warning on `inprocess` + replication.
- A single `Arc<dyn LockManager>` is built in `main.rs` and shared across all
  surfaces. Keys are scoped `{tenant}\0{repo}\0{branch}\0{name}` by callers.

Surfaces:
- **Functions (QuickJS/Starlark)**: `raisin.locks.acquire/release/renew`,
  `raisin.inventory.claim/release`.
- **WS node API**: `locks_acquire`/`locks_release`/`locks_renew`/
  `inventory_claim`/`inventory_release` request types.
- **HTTP**: `POST /api/{repo}/{branch}/locks/{acquire,release,renew}` and
  `/inventory/{claim,release}` (409 on contention / sold-out).

## Code Conventions

- Use `{ workspace = true }` for common dependencies
- Keep files under 300 lines, split into modules as needed
- Use `///` doc comments for public APIs
- Error handling: `raisin-error` types with `thiserror` + `anyhow`
- Async: tokio runtime, `async-trait` for trait methods
