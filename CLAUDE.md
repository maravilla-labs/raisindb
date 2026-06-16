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

## Code Conventions

- Use `{ workspace = true }` for common dependencies
- Keep files under 300 lines, split into modules as needed
- Use `///` doc comments for public APIs
- Error handling: `raisin-error` types with `thiserror` + `anyhow`
- Async: tokio runtime, `async-trait` for trait methods
