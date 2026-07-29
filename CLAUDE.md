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

# Integration tests are consolidated: each crate has ONE test target named `all`
# (crates/<crate>/tests/all/), with one module per former test file. Select by
# module name — the old `--test <file>` form no longer exists.

# Run one former test file (now a module)
cargo test -p raisin-server --test all cluster_social_feed_test -- --ignored --nocapture

# Run a specific test
cargo test -p raisin-server --test all cluster_social_feed_test::test_add_post_node1 -- --ignored --nocapture

# Adding an integration test? Put it in crates/<crate>/tests/all/ and add a
# `mod <name>;` line to that crate's tests/all/main.rs. A new file directly under
# tests/ becomes its own binary again, which is what we consolidated away from.

# Run benchmarks
cargo bench -p raisin-rocksdb

# Quality checks
cargo fmt --workspace
cargo clippy --workspace
```

## Disk: watch `target/`

A full test build is large and *will* fill a disk if unattended. The driver is
structural: **each test target links its own binary**, statically including the
whole dependency graph — rocksdb, tantivy, candle, tesseract — at 25–350 MB apiece.

Integration tests are therefore consolidated to **one target per crate** (see the
`--test all` note above): 108 binaries became 13, cutting test-executable bytes
from 7.3 GB to 2.0 GB and removing 95 link steps. Keep it that way — a new file
placed directly under `tests/` silently adds a binary back.

```bash
make disk          # where target/ actually went, broken down
make prune         # reclaim incremental caches (~5 GB), keeps the library build
make prune-tests   # also drop test executables (~8 GB), keeps the library build
make clean-hard    # cargo clean; full ~20 min rebuild next time
```

Reach for `make prune` first — it is nearly free to regenerate. Only use
`clean-hard` if something is actually corrupt; deleting `target/` wholesale costs
a full rebuild for space you could have reclaimed without one.

Already configured in `Cargo.toml` / `.cargo/config.toml`, so don't re-add:
debug info is `line-tables-only` for workspace crates and off entirely for
dependencies (under **both** the `dev` and `test` profiles — the package override
must be restated for `test`, or dependency debug info returns for all 108
binaries), incremental is off for the `test` profile, and `split-debuginfo=unpacked`
keeps what remains out of the binaries.

The remaining structural lever, not yet taken: consolidating each crate's
integration tests into one binary per crate (`tests/main.rs` with `mod` includes)
would cut 108 binaries to ~13 and save most of that 8 GB, plus a lot of link
time. `raisin-rocksdb` alone has 37.

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

### REFERENCES / hierarchy predicate composition

`REFERENCES('workspace:/path')` (workspace prefix REQUIRED) composes with
`DESCENDANT_OF` / `CHILD_OF` / `node_type` / property predicates, `ORDER BY`,
`LIMIT`, `COUNT(*)`, and bound parameters. The planner prioritizes the
ReferenceIndexScan and applies the rest as residual filters. Two guards keep
this safe (regression: `REFERENCES(...) AND DESCENDANT_OF(...)` used to
silently return zero rows): projection pruning keeps `properties` for any
row-eval REFERENCES/RESOLVE (`raisin-sql/.../projection/column_refs.rs`), and
the row-eval REFERENCES errors loudly if `properties` was pruned. Tests:
`raisin-sql-execution/tests/references_compose_tests.rs`,
`pagination_navigation_tests.rs`.

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

## Editorial Ordering (`__order` / `__tree_order`)

A parent's children carry a **manual order** (drag-and-drop), stored as a
fractional index in the `ORDERED_CHILDREN` CF. Two SQL columns expose it:

- **`__order`** — a node's position among its **siblings**.
- **`__tree_order`** — its position within a **whole subtree** (document order:
  ancestor labels joined, so a node precedes its descendants and subtrees stay
  contiguous). Only tree traversals populate it; NULL elsewhere.

Both are opaque sortable text, and both work as keyset cursors:

```sql
SELECT name, __order FROM 'ws' WHERE CHILD_OF('/menu')
   AND __order > $1 ORDER BY __order LIMIT 20;
SELECT path, __tree_order FROM 'ws' WHERE DESCENDANT_OF('/menu')
   AND __tree_order > $1 ORDER BY __tree_order LIMIT 20;
```

**`__order` is not `path`.** Both order parents before children, but `path` sorts
siblings *alphabetically* while `__order` sorts them *editorially*. They agree
only when the manual order happens to be alphabetical — which is why using `path`
by mistake looks fine until someone drags something. Never mix a cursor on one
with an `ORDER BY` on the other; that drops and duplicates rows.
Test: `raisin-sql-execution/tests/editorial_order_tests.rs`.

Invariants to preserve when touching ordering code:

- **`Node.order_key` is server-assigned and must equal the CF label.** Every path
  that writes `ORDERED_CHILDREN` also stamps the node record — create, update,
  reorder, rebalance, move, cross-branch copy. A reorder therefore produces a node
  revision (visible in history) and replicates. Don't let a client-supplied
  `order_key` through; it is overwritten.
- **Mint labels only via `next_append_label` / `fractional_index::format_label`.**
  Never call `inc()` on a *full* label — it parses hex and chokes on the `::`
  separator, then silently mints a duplicate. Always `extract_fractional` first.
- **Never split an `ORDERED_CHILDREN` key on `\0`.** The descending HLC contains
  null bytes when the counter is 0. Use
  `ordering::key_parse::parse_ordered_child_key`.
- **`DESCENDANT_OF` is pre-order depth-first** (matching table scans). It was BFS
  before; keep parents-before-children, which subtree copy/move/prune rely on.
- Sort elision is gated on `claims_editorial_order`, which the executor honours.
  Do **not** reuse `CompoundIndexScan.pre_sorted` — it's set but ignored.

Docs: `docs/website/docs/access/sql/editorial-ordering.md`.

## Node Revision History & Authorship

**Authorship AND timestamps are stamped at the low-level write layer.**
`created_by` / `updated_by` / `created_at` / `updated_at` are filled in the
low-level write functions every path funnels through: `add_node.rs` (the
optimized CREATE path) and `put_node.rs` (create-or-update), both under
`raisin-rocksdb/.../create/core/`, plus the direct repository path
`repositories/nodes/crud/create/add.rs` (`add_impl`) and `crud/update.rs`.
They resolve the actor from the transaction's auth context (`actor_id()`) →
raw actor → `"anonymous"`. `put_node`/`update` preserve the original
`created_by`/`created_at` on update. Don't re-stamp these in higher layers —
and if you add a new low-level write path, stamp it there too. A missing
`created_at` cascades: no `__created_at` property-index entry → `ORDER BY
created_at LIMIT k` (PropertyOrderScan) silently returns nothing.

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
  surfaces. Scope keys with `raisin_locks::scoped_key(tenant, repo, branch,
  name)` — several call sites still format
  `{tenant}\0{repo}\0{branch}\0{name}` by hand; use the helper in new code.
- **`KeyedMutex` (`raisin-rocksdb/src/jobs/keyed_mutex.rs`) is the in-process
  sibling.** Where `LockManager::try_acquire` REJECTS on contention, `KeyedMutex`
  QUEUES — the right primitive when a second delivery must run after the first,
  not fail. It evicts entries on last release, so a map keyed by something
  unbounded (a flow instance id) doesn't grow forever. Used by the flow-instance
  execution lock and index rebuilds; prefer it over another hand-rolled
  `HashMap<K, Arc<Mutex<()>>>`.

Surfaces:
- **Functions (QuickJS/Starlark)**: `raisin.locks.acquire/release/renew`,
  `raisin.inventory.claim/release`.
- **WS node API**: `locks_acquire`/`locks_release`/`locks_renew`/
  `inventory_claim`/`inventory_release` request types.
- **HTTP**: `POST /api/{repo}/{branch}/locks/{acquire,release,renew}` and
  `/inventory/{claim,release}` (409 on contention / sold-out).

## Workflow Engine (`raisin-flow-runtime`)

Full authoring guide: `docs/workflows.md`. Engine-owned gaps and their triage:
`docs/OPEN-ITEMS.md`. Non-obvious invariants:

- **A flow instance has ONE writer at a time.** `save_instance_with_version` is
  a non-atomic read-check-write (`raisin-rocksdb/.../flow_callbacks/trait_impl.rs`),
  so its version check does NOT stop two concurrent writers — both pass it and
  the second silently overwrites the first. Serialization comes from the
  per-instance lock the job handler takes before any write
  (`jobs/flow_instance_lock.rs`). **Anything that mutates an instance must go
  through that lock** or it can clobber a live execution; `service.rs`'s
  `cancel_instance` writes the node directly and is the one known exception
  (see `docs/OPEN-ITEMS.md`). Multi-node deployments need `[locks]` with the
  `redis` backend — `inprocess`/disabled serializes within ONE node only.
- **Every terminal transition must notify the parent instance.**
  `notify_parent_flow` is called from four places (complete, fail, timeout,
  cancel); miss one and a parent parked on a join waits forever with no error
  anywhere. This has now been the bug twice. The sites are not yet funnelled
  through one helper — check all of them when adding a terminal path.
- **A handler returning `Err` and one returning `StepResult::Error` are the
  same event** and both route through `handle_error_result` (retry / error edge
  / continue-on-fail / fail). Only `FlowError::is_infrastructural()` errors
  (version conflict) return raw, so the job system redelivers. Returning a step
  error raw leaves the instance stuck in `Running` forever.
- **Retry defaults come from the step type**
  (`StepType::default_max_retries`): `parallel`, `sub_flow` and `loop` default
  to 0, because re-entering them re-forks branches or restarts iteration and
  duplicates side effects. Don't reintroduce a flat default.
- **`human_task` properties are ALL template-resolved**, including the numeric
  `due_in_seconds` / `priority` — resolve first, coerce after. Reading them as
  numbers before `DataMapper::map` silently drops any `${...}` value.
- **`task_type` is an open set.** approval/input/review/action are what the
  runtime understands semantically; any `[a-z][a-z0-9_-]{0,63}` slug is valid
  and is carried through verbatim. Validate the SHAPE, never the membership —
  the closed enum used to live in four places (nodetype, runtime, function
  binding, CLI).
- **`parallel` does fork/join AND dynamic fan-out** (`for_each` + a `branch`
  template). The join waits for every child to reach a terminal state, so a
  branch parked on a human task joins correctly. Collection expressions resolve
  through the shared `handlers/collection.rs` — the loop step uses it too.
- **Designer format is the canonical authoring format** and can express every
  step type (`wait`, `sub_flow`, `decision` included). Its siblings chain in
  array order, so a free-standing `decision` is a forward GUARD, not two
  rejoining arms — use an `or` container for mutually exclusive branches.
- Runtime format takes retry config as FLAT step properties (`max_retries`,
  `retry_base_delay_ms`); the nested `retry: {...}` object is designer-format
  only and the converter flattens it.

## Code Conventions

- Use `{ workspace = true }` for common dependencies
- Keep files under 300 lines, split into modules as needed
- Use `///` doc comments for public APIs
- Error handling: `raisin-error` types with `thiserror` + `anyhow`
- Async: tokio runtime, `async-trait` for trait methods
