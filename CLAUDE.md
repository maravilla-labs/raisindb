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

**`cargo check --workspace` does not compile test targets.** Adding a field to a
widely-constructed struct (e.g. `PropertyValueSchema`) passes `check` cleanly and
then fails to build every test that uses a struct literal. Before declaring such
a change green, run `cargo test --workspace --no-run` — or at least
`cargo test -p <crate> --lib --no-run` for the crates you touched.

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
| `raisin-crypto` | AES-256-GCM envelope, `SecretContext` AAD binding, master-key keyring — the ONLY encryptor (see Secrets & Encryption) |
| `raisin-mcp-protocol` | MCP wire types + outbound MCP **client** (no `raisin-functions` dep — see below) |
| `raisin-mcp` | MCP **server** surface; depends on `raisin-mcp-protocol` and re-exports it |

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

## Geospatial: nested geometry & property paths

A geometry may sit **anywhere** in a node's property tree. The index key's
property segment IS the dot path, and the SQL a user writes IS that same string:

| where it sits | SQL | index key segment |
|---|---|---|
| top level | `properties->>'location'` | `location` |
| in an Object | `properties->>'venue.geo'` | `venue.geo` |
| in an Element (section field) | `properties->>'hero.map_pin'` | `hero.map_pin` |
| one array element | `properties->>'stops.0.geo'` | `stops.0.geo` |
| every array element | `properties->>'stops[].geo'` | *not indexed — row scan* |

Docs: `docs/website/docs/access/sql/nested-geospatial.md` and
`geospatial-tracking.md`.

Invariants to preserve:

- **ONE walker, ONE path format.** `indexing/property_walk.rs::walk_properties`
  is the only recursion; `walk_geometries` (spatial), `walk_references`
  (references) and the secret-field selector are selectors over it. A writer and
  a tombstoner that disagree about the format leave entries that can never be
  shadowed — a stale spatial hit surviving every update and delete. Never add a
  second traversal: when a selector needs more information, **extend the one
  walker**. That is why the selector takes a `WalkCursor` (path plus the nearest
  enclosing `element_type`) rather than just the value — value-shaped selection
  works for geometry and references, but not for anything schema-driven.
  `walk_properties_mut` lives in the same file for the same reason, guarded by a
  test asserting it yields exactly the paths `walk_properties` does.
- **The walker descends `Array`, `Object`, `Element` AND `Composite`.** Composite
  was missing until the secret work added it, which meant a geometry inside a
  block collection was stored, reported the index healthy, and was invisible to
  every `ST_DWITHIN`. If you add a new container `PropertyValue` variant, teach
  the walker about it or it silently becomes a hole in three subsystems.
- **A top-level path is byte-identical to the bare property name**, which is why
  no existing index entry needed migrating. Don't "improve" the format.
- **Five sites must walk, not iterate `node.properties`**: the writer and
  tombstoner (`indexing/spatial.rs`), policy resolution
  (`indexing/spatial_policy.rs`), the rebuild job
  (`jobs/handlers/spatial_index.rs`) and the DELETE path
  (`tombstones/index_tombstones.rs`). Miss the last one and a deleted node keeps
  matching forever.
- **The row-level resolver must ship with the index walk.** Before it,
  `properties->>'venue.geo'` was a plain JSON key lookup → NULL → zero rows. That
  is the FALLBACK path, i.e. exactly what runs before a rebuild drains, so
  shipping the index side alone makes the migration window silently EMPTY.
  `eval/functions/geospatial/property_path.rs`.
- **Selection is STRUCTURAL, not shape-driven.** Every stored `Geometry` is
  indexed wherever it sits. The spatial writer is synchronous, takes
  `&mut WriteBatch` and must stay callable from the replication apply path —
  resolving an ElementType there is the read that
  `jobs/handlers/fulltext/batch.rs:152-161` records as reliably deadlocking.
  Shape drives POLICY (precisions, cover), resolved off the hot path.
  Cap: 64 geometry paths per node, warned on overflow.
- **`policy_key_for_path` has exactly ONE implementation**
  (`raisin-models/.../spatial_policy.rs`) and is called from BOTH sides: the
  config surface (`resolve_spatial_policy`, `SpatialWorkspaceSchema::scope`) and
  the local state record (`spatial_state::spatial_state_key`, which the planner's
  `spatial_availability` goes through). It collapses array indices only:
  `stops.3.geo` → `stops[].geo`. If the two sides normalised differently the
  planner would report an indexed field unindexed forever — correct results,
  permanently bad performance, no error anywhere. Resolution is **exact match**;
  there is no prefix inheritance (`venue` does not supply `venue.geo`'s policy).
- **A wildcard path is NEVER index-backed.** Each array element lives in its own
  key namespace, so a scan over `stops[].geo` reads an empty prefix. The guard is
  in `PhysicalPlanner::spatial_availability` — the one call site — which returns
  `Unusable` so every caller inherits it. `spatial_order_is_satisfied` also
  returns false for a wildcard, or keyset pagination drops and duplicates rows.
- **ONE ROW PER NODE**, always. `ST_DWITHIN` over a wildcard is *any*-within;
  `ST_DISTANCE` is the *minimum* (ties → lexicographically smallest concrete
  path). One row per geometry would make a `__distance` cursor straddle rows of
  the same node. `LIMIT k` means k nodes.
- **Tombstone precisions are bounded by `configured ∪ indexed`** when the state
  record was consulted, and widen to all twelve only when it was not. That is
  what takes a tracking profile from 20 writes/update to 4.
- **Nothing bounds superseded-entry accumulation.** Revisions are IN the key, so
  compaction collapses nothing and a rebuild writes MORE tombstones. A tracking
  deployment degrades from ms to seconds within days. The fix is a compaction
  filter on `cf::SPATIAL_INDEX`, deferred — `docs/OPEN-ITEMS.md` §2.99. Do not
  attempt the "seek past a node's remaining revisions" optimisation: within a
  cell the key orders by revision first and `node_id` only as a tiebreak, so all
  nodes' revisions interleave and such a scan silently skips live entries.

## Node Revision History & Authorship

**Timestamps are stamped at every low-level write layer; authorship only where
there is an actor.** The distinction is load-bearing — don't assume a node
written through the repository layer carries `updated_by`.

- **Transaction layer** — `add_node.rs` (the optimized CREATE path) and
  `put_node.rs` (create-or-update), both under
  `raisin-rocksdb/src/transaction/context/nodes/create/core/`. These stamp
  **both** timestamps and authorship, resolving the actor from the transaction's
  auth context (`actor_id()`) → raw actor → `"anonymous"`.
- **Repository layer** — `repositories/nodes/crud/create/add.rs` (`add_impl`) and
  `crud/update.rs` (`update_impl`). These stamp **timestamps only**, via
  `Node::ensure_write_timestamps()`. `update.rs` says so explicitly: *"updated_by
  cannot be resolved here: the repository layer has no actor"*. They do preserve
  the original `created_by`/`created_at` on update.

Don't re-stamp in higher layers — and if you add a new low-level write path,
stamp it there too. A missing `created_at` cascades: no `__created_at`
property-index entry → `ORDER BY created_at LIMIT k` (PropertyOrderScan) silently
returns nothing.

**These four are not an exhaustive content-mutation funnel.**
`repositories/nodes/queries/property.rs` (`update_property`), `publishing.rs`,
`queries/copy/single.rs` and `queries/copy/cross_branch/` all write node content
without going through `put_node`. Anything that must see *every* content mutation
has to cover those too. And `NodeService` is **not** a chokepoint at all: SQL DML
writes straight to the transaction context
(`raisin-sql-execution/.../dml_executor/workspace_dml.rs`,
`bulk_operations.rs`), as does the WS create handler — so logic placed in
`NodeService` is silently bypassed by `psql` and by WebSocket writes.

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

## Secrets & Encryption

**Plaintext never leaves. Ciphertext is not a secret only because the key is not
in the database** — that is what makes a stolen RocksDB backup inert, and it is
why the root key stays outside the DB.

A schema field declares `encrypted: true` (a first-class field on
`PropertyValueSchema` and `FieldTypeSchema`; the legacy `meta.secret: true`
spelling is still read as a fallback — one reader, `is_secret`, so the two cannot
drift). On write the server moves the value into `cf::SECRETS` and rewrites the
property to a `secret://<name>[@<version>]` reference. **Reads never resolve**;
they return the reference, which is why a reference is safe in a node property,
an API response, a SQL result, an audit entry and a replication payload.

- **The write layer is the enforcement point, NOT `NodeService`.** SQL DML and
  the WS create handler write straight to the transaction context, so anything
  placed in `NodeService` is bypassable by `psql`. Vaulting hooks
  `transaction/context/nodes/create/core/{put_node,add_node}.rs`, plus the paths
  that never see a transaction (`repositories/nodes/queries/property.rs`'s
  `update_property`, and `crud/create/add.rs` / `crud/update.rs` as backstops).
- **Vault BEFORE indexing.** Index first and the plaintext's hash is written to
  `cf::PROPERTY_INDEX` / `UNIQUE_INDEX` / `COMPOUND_INDEX` permanently, which
  turns `properties->>'password'::String = '<guess>'` into an oracle. Ordering
  inside `put_node`: after revision allocation and `validate_node` (so the
  validator still sees plaintext and its constraints mean something), before the
  read cache, the node blob write, and every index call.
- **Fail CLOSED.** Every neighbouring module fails open — `coercion.rs` swallows
  schema-resolution errors, the fulltext planner falls back to index-all-strings.
  Copying that here writes plaintext to disk. On any doubt, refuse the write.
- **The `has encrypted fields` gate is a cache, and a stale `false` is a plaintext
  leak** — so it is event-invalidated on `Event::Schema` (never TTL, which would
  be a bounded leak window) *and* registered with `derived_cache_registry`,
  because checkpoint SST ingest emits no events. Stale `true` costs one wasted
  walk; stale `false` costs a secret. Bias to `true`.
- **The replication apply path must not re-vault** — an arriving node already
  carries references. It does not go through `put_node`, so this is free; keep it
  that way.
- **`SecretContext` fields are private, constructible only via family
  constructors** in `raisin-crypto`. Its AAD binds ciphertext to
  `{tenant, repo, scope, field}`, length-prefixed (not `\0`-joined, or
  `tenant="a\0b"` would collide with `tenant="a", repo="b"`). Several readers
  swallow decrypt errors with `.ok()`, so a hand-written context that drifts from
  the writer's does not throw — it silently becomes "credential absent, proceed
  UNAUTHENTICATED".
- **Envelope v2 is `[RSB2][key_id u16][nonce 12][ct+tag]`; the reader takes v1
  forever.** `seal()` still EMITS v1 unless `RAISIN_CRYPTO_EMIT_V2` is set,
  because a v2 blob is unreadable by a peer on the previous binary — the format
  cannot flip in the same release that learns to read it. Where v1 is accepted the
  AAD is accident-detection, not attacker-resistance: stripping v2→v1 is deleting
  six bytes.
- **Key ids are reserved**: `0x0000` legacy `RAISIN_MASTER_KEY`, `0xFFFE` the
  insecure dev zero key, `0xFFFF` per-process ephemeral. Opening a `0xFFFE` blob
  outside dev mode is an ERROR naming the dev key, so a dev database promoted to
  prod fails loudly instead of looking encrypted.
- **`cf::SECRETS` is branch-scoped and `Copied` on fork**, keyed
  `{tenant}\0{repo}\0{branch}\0{name}\0{~rev}` with a descending HLC last
  (`RevisionLocator::Tail`); `{name}` must be null-free. A reference is
  branch-agnostic while the store is branch-scoped, so **cross-branch promotion
  must copy the referenced secret rows** — otherwise publish produces a fork whose
  nodes render fine until someone reveals one. A node COPY must re-vault, or
  deleting the source destroys the copy's secret.
- **Delete tombstones, never refcounts.** Older node revisions still reference
  older versions, and refcounting would mean indexing every revision of every node
  on every branch — a second reference index with a second path format.
- Adding a column family means: `mod cf`, `all_column_families()`,
  `BRANCH_CF_REGISTRY` (a test fails the build if unclassified) **and**
  `TENANT_PREFIXED_CFS` in `storage/tenant_wipe.rs` — that last one is *not*
  test-enforced and is the one that gets missed.

Not yet done, deliberately: existing `*_encrypted` node properties are unmigrated
and still visible to `SELECT *`; `Permission::is_field_accessible` remains dead
code; SQL DDL has no `ENCRYPTED` modifier, so secret fields must be declared in
YAML.

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
- **Every terminal transition must notify the parent instance.** Miss one and a
  parent parked on a join waits forever with no error anywhere. This has now been
  the bug twice. Four terminal paths, but only **three** of them call
  `notify_parent_flow`:
  - complete — `runtime/executor/result_handlers.rs` (`"completed"`)
  - fail — `runtime/executor/result_handlers.rs` (`"failed"`)
  - timeout — `runtime/resume/mod.rs` (`"failed"`)
  - cancel — `service.rs::cancel_instance` **hand-rolls the same
    `function_result` / `child_completed` payload inline** instead of calling the
    helper.

  So changing the payload shape in `notify_parent_flow` silently diverges the
  cancel path. Funnelling cancel through the helper is the obvious fix and has not
  been done; until then, treat the two as a pair.
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
  numbers before `DataMapper::map` silently drops any `${...}` value. **The
  DESIGNER types must also be loose enough** (`TemplatableNumber`): they are a
  separate, stricter declaration, and a narrow `Option<i64>` there makes a
  template a DESERIALIZATION error that takes the whole flow definition down
  (`invalid type: string, expected i64`). Fixing only the runtime format is
  half a fix.
- **Node events come from `NodeService` and the flow node callbacks, NOT from
  the raw node repository.** A write via `storage.nodes().update(...)` is
  silent — no `node:updated`, so WS subscriptions, triggers and indexing never
  see it. That is how task completion ended up unobservable while creation and
  expiry were fine. Either go through the callbacks or publish on
  `storage.event_bus()` explicitly.
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

## MCP: both directions

RaisinDB is both an MCP **server** (external clients call its tools) and an MCP
**client** (its agents call other servers' tools).

**The crate split is load-bearing, not cosmetic.** `raisin-mcp` serves tools, so
it depends on `raisin-functions` — which depends on `raisin-rocksdb`. Anything at
or below the storage layer therefore cannot depend on `raisin-mcp` without
closing a package cycle, and **Cargo rejects package cycles regardless of
features**. `raisin-mcp-protocol` holds the wire types and the client with no
`raisin-functions` dependency; `raisin-mcp` re-exports it, so
`raisin_mcp::protocol::…` paths still resolve. Do not "simplify" them back
together.

Invariants to preserve:

- **A proxy is a `raisin:Function` with an `mcp_proxy` block.** Presence of that
  block is the discriminator, checked in `execute_function`
  (`raisin-functions/.../executor.rs`) BEFORE the code load. It cannot be
  `execution_mode` (an unknown value silently parses as `Async`) nor `language`
  (an unknown value is a hard error). `mcp` on a function means the OPPOSITE
  direction — expose this local function to inbound clients.
- **One execution branch, not two.** All three paths (JS chat via the
  `AIToolCall` job, `raisin.functions.execute()`, the flow runtime) funnel
  through `execute_function`. Add remote-tool behaviour there, never per-path.
- **Proxy paths never change.** An agent's `tools:` array holds a path; a rename
  makes the tool vanish from that agent with no error anywhere. Slugs are
  derived deterministically and collision suffixes assigned in sorted
  remote-name order, so a server that reorders its listing cannot renumber them.
- **A steady-state discovery writes nothing.** The schema-hash guard in
  `reconcile_plan` is what stops an hourly refresh minting thousands of function
  revisions a year. The hash is over CANONICAL JSON — this workspace builds
  `serde_json` with `preserve_order`, so key order alone would otherwise change
  the hash and rewrite every proxy forever.
- **A tool that disappears upstream is disabled, never deleted.** A missing node
  makes the tool vanish silently; a disabled one stays visible in the console.
- **A failed probe records health and leaves proxies alone.** A remote being
  down means the tool list is unknown, not empty.
- **Egress is checked twice** — when a connection is saved and again before
  every dial — because a host that resolved publicly can be re-pointed later.
  Config is `[mcp_client]`, installed process-wide via
  `raisin_functions::configure_mcp_client`, so every path shares one policy.
- **`tools/call` is never auto-retried.** MCP has no idempotency key; a retry
  can charge a card twice. Only session recovery replays, once.

## Functions never reach loopback — platform hooks do

`raisin.http.fetch` refuses loopback, private and link-local addresses for
EVERY function (`raisin-functions/.../callbacks/http.rs`, an `EgressPolicy`
check before the request and inside DNS resolution). Do not add an allowlist
there: it would open the address for every tenant's content code, not just
ours. When a function must reach a platform service on such an address
(Flightdeck's "install the new Studio package", for one), the operator names
the endpoint in the server config and the function calls it BY NAME:

```toml
[platform.hooks.studio_update]
url = "http://127.0.0.1:8080/internal/studio/update"
token_env = "STUDIO_INTERNAL_TOKEN"
token_header = "x-studio-internal-token"
```

```js
const r = await raisin.platform.hook('studio_update', {}); // { ok, status, body }
```

The runtime stamps `tenant_id` / `repo_id` into the payload (a function cannot
fire a hook for another tenant), the token never passes through tenant data,
and the call goes through the shared client, not the guarded one. Same pattern
as the media/screenshot plugin bindings — trusted server code on a known
address. Implementation: `execution/callbacks/platform.rs`; config plumbing
`raisin-server/src/config.rs` (`PlatformConfig`) → `startup/cli.rs` →
`main.rs` (`configure_platform_hooks`).

## Job dedup is per-PROCESS, not per-cluster

`JobRegistry`'s dedup map is an in-memory `HashMap` with no storage behind it
(`raisin-storage/src/jobs/registry/`; `register_job_with_id_idempotent` lives in
`registry/registration.rs`). It collapses duplicates **within one process only** —
on an N-node cluster every node runs its own copy of a periodic job.

Anything that must run once per cluster needs a `raisin_locks` lease inside the
handler (and the `redis` backend; `inprocess` serializes within one node).
Both the integration token refresh and MCP tool discovery take a per-entity
lease for exactly this reason — without it, two nodes presenting the same
rotating OAuth refresh token can invalidate each other's, and two nodes writing
the same node lose one another's update.

## Code Conventions

- Use `{ workspace = true }` for common dependencies
- Keep files under 300 lines, split into modules as needed
- Use `///` doc comments for public APIs
- Error handling: `raisin-error` types with `thiserror` + `anyhow`
- Async: tokio runtime, `async-trait` for trait methods
