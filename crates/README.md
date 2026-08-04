# RaisinDB Crates

This directory contains all the Rust crates that make up RaisinDB.

## Core Crates

### `raisin-core`
Business logic and service layer. Contains:
- `NodeService`: CRUD operations for nodes
- `WorkspaceService`: Workspace management
- Validation and auditing logic

### `raisin-models`
Data models and types:
- Node structures
- NodeType schemas
- Property definitions
- Validation rules

### `raisin-storage`
Storage abstraction traits:
- `Storage`: Main storage trait
- `NodeRepository`: Node storage operations
- `NodeTypeRepository`: NodeType storage operations
- `ScopedStorage`: Multi-tenant wrapper

## Storage Implementations

### `raisin-storage-rocks`
RocksDB-backed storage (default):
- High performance
- Persistent storage
- Production-ready

### `raisin-storage-memory`
In-memory storage:
- Fast for testing
- No persistence
- Development use

### `raisin-storage-mongodb` (planned)
MongoDB backend for flexible schemas

### `raisin-storage-postgres` (planned)
PostgreSQL backend for relational queries

## Multi-Tenancy

### `raisin-context`
Multi-tenancy types and traits:
- `TenantContext`: Tenant identification
- `TenantResolver`: Extract tenant from requests
- `TierProvider`: Service tier management
- `RateLimiter`: Rate limiting interface

### `raisin-ratelimit`
Rate limiting implementations:
- RocksDB-backed rate limiter
- Sliding window algorithm
- Per-tenant limits

## Supporting Crates

### `raisin-error`
Error types and Result aliases

### `raisin-audit`
Audit logging:
- Track all changes
- In-memory and persistent backends

### `raisin-versioning`
Version control for nodes:
- Track changes over time
- Restore previous versions

### `raisin-binary`
Binary/file storage:
- Filesystem backend
- S3 backend
- Multi-tenant path prefixing

### `raisin-events`
Event system for real-time updates

### `raisin-indexer`
Search indexing and querying

### `raisin-query`
Query language and execution

### `raisin-i18n`
Internationalization support

### `raisin-scripting-lua`
Lua scripting integration

## Model Context Protocol

### `raisin-mcp-protocol`
MCP wire types and the outbound **client** — RaisinDB calling somebody else's
MCP server:
- JSON-RPC envelopes and typed `tools/*` / `resources/*` payloads
- Streamable HTTP transport, session cache, egress (SSRF) policy
- Tool discovery reconcile planner
- Deliberately does NOT depend on `raisin-functions`, so storage-layer job
  handlers can use MCP without a dependency cycle

### `raisin-mcp`
The MCP **server** surface — serving RaisinDB's own tools to external clients:
- Tool registry, data tools, dispatcher, MCP-UI (SEP-1865) widgets
- Servers are content: a `raisin:McpServer` node, not config
- Depends on `raisin-mcp-protocol` and re-exports it

## Transport Layers

### `raisin-transport-http`
HTTP/REST API:
- Axum-based server
- Repository-style routes
- Multi-tenant middleware support

### `raisin-transport-ws`
WebSocket support for real-time

### `raisin-transport-inprocess`
Direct in-process communication

## Application Crates

### `raisin-server`
Reference HTTP server implementation

### `raisin-client`
Client library for connecting to RaisinDB

## Dependency Graph

```
raisin-server
  ├─ raisin-transport-http
  │   ├─ raisin-core
  │   │   ├─ raisin-storage
  │   │   │   └─ raisin-context
  │   │   ├─ raisin-models
  │   │   └─ raisin-error
  │   ├─ raisin-audit
  │   └─ raisin-binary
  └─ raisin-storage-rocks
      └─ raisin-storage

raisin-ratelimit
  ├─ raisin-context
  └─ rocksdb
```

## Building

Build all crates:
```bash
cargo build --workspace
```

Build specific crate:
```bash
cargo build --package raisin-core
```

Run tests:
```bash
cargo test --workspace
```

## Adding a New Crate

1. Create directory: `crates/raisin-newcrate`
2. Add `Cargo.toml` with workspace dependencies
3. Add to workspace members in root `Cargo.toml`
4. Follow project structure conventions
5. Add README explaining the crate
6. Update this file

## Conventions

- Use `{ workspace = true }` for common dependencies
- Keep files under 300 lines
- Split into modules as needed
- Add tests in `#[cfg(test)]` modules or `tests/` directory
- Document public APIs with `///` doc comments
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
