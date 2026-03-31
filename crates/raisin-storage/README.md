# raisin-storage

Storage trait definitions and abstractions for RaisinDB.

## Overview

This crate defines the storage abstraction layer that enables pluggable storage backends. All storage implementations (RocksDB, PostgreSQL, in-memory) conform to these traits, providing a consistent interface for data persistence.

- **Trait-Based Abstraction**: Pluggable backends through trait implementation
- **Multi-Tenant Isolation**: Explicit tenant_id, repo_id, branch parameters
- **MVCC Support**: HLC timestamps for time-travel queries and snapshot isolation
- **Rich Indexing**: Property, reference, spatial, compound, and full-text indexes
- **Transaction Support**: Atomic multi-step operations with commit/rollback
- **Background Jobs**: Priority-based job queue with monitoring and persistence

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Storage Trait                            │
│  - NodeRepository      - PropertyIndexRepository            │
│  - WorkspaceRepository - ReferenceIndexRepository           │
│  - NodeTypeRepository  - SpatialIndexRepository             │
│  - BranchRepository    - CompoundIndexRepository            │
│  - RevisionRepository  - FullTextJobStore                   │
└──────────────────────────┬──────────────────────────────────┘
                           │
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│   RocksDB    │  │  PostgreSQL  │  │   In-Memory  │
│  (primary)   │  │   (planned)  │  │   (testing)  │
└──────────────┘  └──────────────┘  └──────────────┘
```

## Core Traits

### Storage

Main entry point providing access to all repositories:

```rust
pub trait Storage: Send + Sync {
    type Nodes: NodeRepository;
    type NodeTypes: NodeTypeRepository;
    type Workspaces: WorkspaceRepository;
    type PropertyIndex: PropertyIndexRepository;
    // ... more repositories

    fn nodes(&self) -> &Self::Nodes;
    fn begin(&self) -> Result<Box<dyn Transaction>>;
    fn event_bus(&self) -> Arc<dyn EventBus>;
}
```

### NodeRepository

CRUD operations for nodes with tree management:

```rust
pub trait NodeRepository: Send + Sync {
    // Core CRUD
    fn get(&self, tenant_id, repo_id, branch, workspace, id, max_revision) -> Result<Option<Node>>;
    fn create(&self, ..., options: CreateNodeOptions) -> Result<()>;
    fn update(&self, ..., options: UpdateNodeOptions) -> Result<()>;
    fn delete(&self, ..., options: DeleteNodeOptions) -> Result<bool>;

    // Tree operations
    fn list_by_parent(&self, ..., options: ListOptions) -> Result<Vec<Node>>;
    fn scan_by_path_prefix(&self, ..., path_prefix: &str) -> Result<Vec<Node>>;
    fn move_node(&self, ..., new_path: &str) -> Result<()>;
    fn copy_node_tree(&self, ...) -> Result<Node>;

    // Publishing workflow
    fn publish(&self, ...) -> Result<()>;
    fn unpublish(&self, ...) -> Result<()>;
}
```

## Repositories

| Repository | Purpose |
|------------|---------|
| `NodeRepository` | Node CRUD, tree operations, publishing |
| `NodeTypeRepository` | Schema definitions for nodes |
| `WorkspaceRepository` | Workspace management |
| `PropertyIndexRepository` | Fast property value lookups |
| `ReferenceIndexRepository` | Forward/reverse reference tracking |
| `SpatialIndexRepository` | Geohash-based proximity queries |
| `CompoundIndexRepository` | Multi-column index queries |
| `BranchRepository` | Git-like branch operations |
| `RevisionRepository` | Immutable revision tracking |
| `TranslationRepository` | Multi-language content storage |
| `FullTextJobStore` | Full-text indexing job queue |

## Features

### Transactions

```rust
let tx = storage.begin().await?;

// Perform multiple operations atomically
storage.nodes().create(...).await?;
storage.nodes().update(...).await?;

tx.commit().await?;
// or tx.rollback().await?;
```

### Time-Travel Queries (MVCC)

```rust
// Query at specific revision using HLC timestamp
let node = storage.nodes()
    .get(tenant, repo, branch, workspace, id, Some(&max_revision))
    .await?;
```

### Performance Controls

```rust
// API calls: compute has_children for UI tree display
let nodes = storage.nodes()
    .list_by_parent(..., ListOptions::for_api())
    .await?;

// SQL queries: skip has_children for performance
let nodes = storage.nodes()
    .list_by_parent(..., ListOptions::for_sql())
    .await?;
```

### Background Jobs

```rust
use raisin_storage::jobs::{JobRegistry, JobType, JobStatus};

// Register a job
let job_id = JobRegistry::register_job(
    JobType::FulltextIndex,
    "Indexing workspace documents",
    tenant_id,
).await?;

// Check status
let status = JobRegistry::get_status(&job_id).await?;
```

## Modules

| Module | Description |
|--------|-------------|
| `jobs/` | Background job management (registry, worker pool, monitor) |
| `fulltext.rs` | Full-text search indexing abstractions |
| `spatial.rs` | Geohash-based geospatial indexing |
| `transactional.rs` | Transaction context for atomic operations |
| `translations.rs` | Multi-language translation storage |
| `management.rs` | Integrity checks, backups, repairs |
| `system_updates.rs` | NodeType/Workspace version tracking |
| `node_operations.rs` | Create/Update/Delete options |
| `repository.rs` | Branch, tag, revision management |

## Key Types

```rust
// Node operation options
pub struct CreateNodeOptions {
    pub validate_schema: bool,
    pub auto_create_parents: bool,
}

pub struct ListOptions {
    pub compute_has_children: bool,  // Performance control
}

// Job management
pub enum JobType {
    IntegrityScan, IndexRebuild, FulltextIndex,
    AssetProcessing, /* ... 30+ variants */
}

pub enum JobStatus {
    Pending, Running, Completed, Failed, Cancelled,
}

// Spatial indexing
pub struct SpatialIndexEntry {
    pub node_id: String,
    pub geohash: String,
    pub lat: f64,
    pub lon: f64,
}
```

## Dependencies

**Internal crates:**
- `raisin-models` - Core data models (Node, Workspace, etc.)
- `raisin-error` - Error types
- `raisin-hlc` - Hybrid Logical Clock for MVCC
- `raisin-context` - TenantContext, IsolationMode
- `raisin-events` - Event bus for change notifications

**External crates:**
- `tokio` - Async runtime
- `async-trait` - Async trait support
- `dashmap` - Concurrent HashMap for job registry

## Used By

- `raisin-core` - Core services layer
- `raisin-sql-execution` - SQL query execution
- `raisin-rocksdb` - RocksDB storage implementation
- `raisin-storage-memory` - In-memory storage for testing
- `raisin-indexer` - Full-text search indexing
- `raisin-functions` - Serverless function execution
- `raisin-transport-http` - HTTP API handlers

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
