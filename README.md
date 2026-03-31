# RaisinDB

**The Multi-Model Database With Git-like Workflows**

PostgreSQL-compatible SQL · Graph · Vector Search · Full-Text · Real-Time Events

[![License: BSL-1.1](https://img.shields.io/badge/License-BSL--1.1-blue.svg)](./LICENSE)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](https://github.com/maravilla-labs/raisindb/releases)

## What is RaisinDB?

RaisinDB is a multi-model database combining document storage, graph relationships, vector search, and full-text indexing with Git-like version control (branches, merges, time-travel). Schemas are defined declaratively via YAML NodeTypes that control validation, indexing, and query behavior. Multi-tenant by design with key-based isolation. Built in Rust on RocksDB, it ships as a single binary accessible via HTTP REST, WebSocket, and PostgreSQL wire protocol.

## Quick Start

### Install via npm (recommended)

```bash
npm install -g @raisindb/cli
raisindb server start
```

The CLI downloads the right server binary for your platform (macOS, Linux, Windows) and starts it.

### Or build from source

```bash
cargo build --release -p raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/release/raisin-server --pgwire-enabled true
```

### Connect and query

```bash
# Connect with psql
psql -h localhost -p 5432
```

```sql
-- Create a node
INSERT INTO 'workspace' (name, node_type, parent_path, properties)
VALUES ('hello-world', 'post', '/', '{"title": "Hello", "body": "First post"}');

-- Query with property access
SELECT name, properties->>'title'::String AS title
FROM 'workspace'
WHERE node_type = 'post';

-- Graph traversal: find neighbors
SELECT * FROM NEIGHBORS('workspace', 'node-id', 'OUTBOUND');

-- Full-text search
SELECT * FROM 'workspace' WHERE SEARCH(properties->>'body'::String, 'hello');

-- Create a branch
CREATE BRANCH 'feature/new-design' FROM 'main';

-- Time-travel query
SELECT * FROM 'workspace' AT REVISION 42;
```

## Installation

| Method | Command |
|--------|---------|
| **npm CLI** (recommended) | `npm install -g @raisindb/cli && raisindb server start` |
| **Binary download** | [GitHub Releases](https://github.com/maravilla-labs/raisindb/releases) |
| **From source** | `cargo build --release -p raisin-server --features "storage-rocksdb,websocket,pgwire"` |

The CLI also provides package management and development tools:

```bash
raisindb server install     # Download server binary
raisindb server start       # Start the server (auto-installs if needed)
raisindb server update      # Update to latest version
raisindb package create .   # Create a RAP package
raisindb package sync       # Sync local changes to server
raisindb shell              # Interactive SQL shell
```

## Core Features

| Category | Features |
|----------|----------|
| **Query Languages** | PostgreSQL-compatible SQL, SQL/PGQ graph queries, OpenCypher |
| **Data Models** | Documents (hierarchical), Graph (bidirectional edges), Vector (HNSW), Full-text (Tantivy) |
| **Version Control** | Branches, tags, commits, merges, time-travel queries |
| **Schema** | YAML NodeTypes with per-property indexing, validation, fulltext/SQL flags |
| **Multi-Tenancy** | Key-based tenant isolation, per-tenant auth, rate limiting |
| **Functions** | Embedded JavaScript (QuickJS) and Starlark runtimes, event triggers, cron, HTTP invocation |
| **Auth** | Pluggable strategies (local, OIDC, magic link, API keys), per-tenant config |
| **Real-Time** | WebSocket event streaming, live subscriptions |
| **Clustering** | CRDT-based multi-master replication, masterless, eventual consistency |
| **Access Protocols** | HTTP REST API, WebSocket, PostgreSQL wire protocol (psql-compatible) |
| **Translations** | Built-in i18n with locale overlays, fallback chains |
| **Storage** | RocksDB with 40+ column families, in-memory backend for testing |

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        raisin-server                             │
├──────────────┬──────────────────┬────────────────────────────────┤
│  HTTP/REST   │    WebSocket     │   PostgreSQL Wire Protocol     │
│  (Axum)      │    (tokio-ws)    │   (pgwire)                    │
├──────────────┴──────────────────┴────────────────────────────────┤
│                     Core Business Logic                          │
│         (NodeService, WorkspaceService, Validation)              │
├────────────┬──────────────┬──────────────┬───────────────────────┤
│  SQL/PGQ   │  Functions   │  Replication │  Auth                 │
│  Execution │  (QuickJS /  │  (CRDT)      │  (OIDC / Local /     │
│  (Tantivy/ │   Starlark)  │              │   Magic Link)        │
│   HNSW)    │              │              │                       │
├────────────┴──────────────┴──────────────┴───────────────────────┤
│                      Storage Abstraction                         │
├──────────────────────────────┬───────────────────────────────────┤
│          RocksDB             │          In-Memory                │
│    (40+ column families)     │        (testing)                  │
└──────────────────────────────┴───────────────────────────────────┘
```

## Project Structure

```
crates/
├── raisin-server/           # Server binary, config, migrations
├── raisin-core/             # Business logic, NodeService, WorkspaceService
├── raisin-models/           # Data types (Node, NodeType, PropertyValue)
├── raisin-storage/          # Storage trait abstraction
├── raisin-rocksdb/          # RocksDB implementation, job queue
├── raisin-sql/              # SQL parser, analyzer, logical planner
├── raisin-sql-execution/    # Physical plan execution, PGQ support
├── raisin-cypher-parser/    # OpenCypher query parser
├── raisin-indexer/          # Full-text indexing (Tantivy)
├── raisin-hnsw/             # Vector similarity search (HNSW)
├── raisin-functions/        # Embedded function runtimes (QuickJS, Starlark)
├── raisin-auth/             # Authentication strategies
├── raisin-replication/      # CRDT-based multi-master replication
├── raisin-hlc/              # Hybrid Logical Clock
├── raisin-transport-http/   # Axum REST API handlers
├── raisin-transport-ws/     # WebSocket transport
└── raisin-transport-pgwire/ # PostgreSQL wire protocol
packages/
├── raisindb-cli/            # CLI tool (@raisindb/cli)
├── raisin-client-js/        # JavaScript/TypeScript client (@raisindb/client)
└── admin-console/           # Web-based admin UI
```

## SQL Examples

### Property Queries

```sql
SELECT name, properties->>'email'::String AS email
FROM 'workspace'
WHERE properties->>'role'::String = 'admin';
```

### Graph Traversal

```sql
-- SQL/PGQ style
SELECT * FROM GRAPH_TABLE (
  MATCH (a)-[e:follows]->(b)
  WHERE a.name = 'alice'
  COLUMNS (b.name AS friend)
);

-- Neighbor lookup
SELECT * FROM NEIGHBORS('workspace', 'user-123', 'OUTBOUND')
WHERE edge_type = 'follows';
```

### Vector Similarity

```sql
SELECT name, DISTANCE(embedding, [0.1, 0.2, 0.3]) AS score
FROM 'workspace'
WHERE node_type = 'document'
ORDER BY score ASC
LIMIT 10;
```

### Full-Text Search

```sql
SELECT name, properties->>'title'::String
FROM 'workspace'
WHERE SEARCH(properties->>'body'::String, 'database AND graph');
```

## Git-like Workflows

```sql
-- Create a branch from main
CREATE BRANCH 'feature/redesign' FROM 'main';

-- Work on the branch (switch context)
-- ... make changes ...

-- Time-travel: query data at a specific point
SELECT * FROM 'workspace' AT REVISION 100;

-- Merge branch back
MERGE BRANCH 'feature/redesign' INTO 'main';
```

## Development

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run cluster integration tests
cargo test --package raisin-server --test cluster_social_feed_test -- --ignored --nocapture

# Format & lint
cargo fmt --workspace
cargo clippy --workspace

# Run benchmarks
cargo bench -p raisin-rocksdb
```

Start a 3-node test cluster:

```bash
./scripts/start-cluster.sh
```

## Documentation

- [Documentation Website](https://raisindb.com) — Guides, tutorials, and API reference
- [mdBook Documentation](./book/) — Getting started, architecture deep-dives
- [Crate READMEs](./crates/) — Per-crate documentation

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines. By contributing, you agree to the [CLA](./CLA.md).

## License

[Business Source License 1.1](./LICENSE) — Licensed by Maravilla Labs (SOLUTAS GmbH). Converts to Apache 2.0 four years after first public release.
