# raisin-rocksdb

RocksDB storage backend implementation for RaisinDB - the heart of the storage system.

## Overview

This crate implements the storage layer for RaisinDB using RocksDB as the underlying engine. It provides:

- **Multi-tenant Storage**: Complete isolation between tenants with shared infrastructure
- **Revision-Aware Indexing**: MVCC support with descending revision encoding for time-travel queries
- **Repository Pattern**: 25+ specialized repositories for different data types
- **Transaction Support**: ACID transactions with automatic conflict detection
- **Replication Integration**: CRDT operation capture and replay for clustering
- **Background Jobs**: Async operation queue for deferred processing

## Key Features

| Feature | Description |
|---------|-------------|
| **40+ Column Families** | Optimized storage layout for different data types |
| **MVCC Time-Travel** | Query any revision with `get_at_revision()`, `list_at_revision()` |
| **Row-Level Security** | REL expression evaluation for access control |
| **Full-Text Search** | Tantivy integration with lazy indexing |
| **Vector Search** | HNSW index support for embeddings |
| **Spatial Indexing** | Geohash-based PostGIS-compatible ST_* queries |
| **Identity System** | Multi-provider authentication with sessions |
| **Graph Operations** | Relation storage with graph algorithm support |

## Architecture

### Storage Hierarchy

```
RocksDBStorage (central abstraction)
├── Config & Connection Pool
├── Transaction Management (RocksDBTransaction)
├── Column Family Access
└── Repository Factories
    ├── Core Repositories
    │   ├── NodeRepository (nodes, properties, indexes)
    │   ├── RelationRepository (graph edges)
    │   ├── BranchRepository (Git-like branching)
    │   ├── WorkspaceRepository (draft state)
    │   └── TranslationRepository (i18n)
    ├── Schema Repositories
    │   ├── NodeTypeRepository
    │   ├── ArchetypeRepository
    │   └── ElementTypeRepository
    ├── Index Repositories
    │   ├── PropertyIndexRepository
    │   ├── SpatialIndexRepository
    │   └── UniqueIndexRepository
    ├── Auth Repositories
    │   ├── IdentityRepository
    │   ├── SessionRepository
    │   └── AdminUserStore
    └── System Repositories
        ├── OpLogRepository (replication)
        ├── JobDataStore (background jobs)
        └── EmbeddingStorage (vectors)
```

### Column Families

The storage uses 40+ column families for optimized access patterns:

**Core Data:**
- `nodes` - Node blobs with revision-aware keys
- `path_index` - Hierarchical path navigation
- `property_index` - Property value indexes with bloom filters
- `relation_index` - Graph edge storage
- `ordered_children` - Sibling ordering with prefix optimization

**Schema:**
- `node_types`, `archetypes`, `element_types`

**Versioning:**
- `branches`, `tags`, `revisions`, `trees`
- `workspaces`, `workspace_deltas`

**Search:**
- `fulltext_jobs` - Tantivy indexing queue
- `embeddings`, `embedding_jobs` - Vector storage
- `spatial_index` - Geohash-based spatial queries
- `compound_index` - Multi-column indexes

**Auth:**
- `identities`, `identity_email_index`, `sessions`
- `admin_users`, `tenant_auth_config`

**Replication:**
- `operation_log` - CRDT operation log
- `applied_ops` - Idempotency tracking

**Jobs:**
- `job_data`, `job_metadata` - Background job queue

### Key Encoding

Keys use a hierarchical structure with null-byte delimiters:

```
{tenant}\0{repo}\0{branch}\0{workspace}\0{entity}\0{id}\0{~revision}
```

- **Descending revision**: `~revision = u64::MAX - revision` for efficient "latest" queries
- **Tombstone pattern**: Deleted entities marked with tombstone at revision, not removed
- **Prefix scans**: RocksDB prefix iterators for efficient range queries

## Quick Start

### Basic Usage

```rust
use raisin_rocksdb::{RocksDBStorage, RocksDBConfig};

// Open with development config
let storage = RocksDBStorage::open("/var/lib/raisindb")?;

// Or with production config
let config = RocksDBConfig::production().with_path("/var/lib/raisindb");
let storage = RocksDBStorage::open_with_config(config)?;
```

### Transaction Example

```rust
use raisin_rocksdb::RocksDBTransaction;

// Start a transaction
let tx = storage.begin_transaction()?;

// Perform operations
tx.create_node(&tenant, &repo, &branch, &workspace, node)?;
tx.set_property(&tenant, &repo, &branch, &workspace, &node_id, "title", value)?;

// Commit atomically
tx.commit()?;
```

### Time-Travel Queries

```rust
// Get node at specific revision
let node = storage.get_at_revision(&tenant, &repo, &branch, &node_id, revision)?;

// Get node history
let history = storage.get_history(&tenant, &repo, &branch, &node_id, limit)?;

// List children at revision
let children = storage.list_at_revision(&tenant, &repo, &branch, &parent_id, revision)?;
```

## Configuration

### RocksDBConfig

```rust
use raisin_rocksdb::{RocksDBConfig, CompressionType};

let config = RocksDBConfig {
    path: "/var/lib/raisindb".into(),

    // Performance tuning
    max_open_files: 10000,
    write_buffer_size_mb: 64,
    max_write_buffer_number: 3,
    target_file_size_base_mb: 64,

    // Parallelism
    max_background_jobs: 8,

    // Compression
    compression: CompressionType::Lz4,

    // Bloom filters (automatic for property_index, spatial_index, unique_index)
    ..Default::default()
};
```

### Preset Configurations

```rust
// Development (fast startup, low memory)
let config = RocksDBConfig::development();

// Production (optimized for throughput)
let config = RocksDBConfig::production();
```

## Replication Integration

### Operation Capture

Operations are captured for CRDT replication:

```rust
use raisin_rocksdb::OperationCapture;

// Enable operation capture
let capture = OperationCapture::new(storage.clone());

// Operations automatically recorded to oplog
tx.create_node(...)?;
tx.commit()?;

// Get operations since vector clock
let ops = capture.get_operations_since(&tenant, &repo, &since_vc, limit).await?;
```

### Checkpoint Transfer

For catch-up synchronization:

```rust
use raisin_rocksdb::{CheckpointManager, CheckpointReceiver};

// Create checkpoint
let manager = CheckpointManager::new(storage.clone());
let metadata = manager.create_checkpoint().await?;

// Transfer to new node
let receiver = CheckpointReceiver::new("/path/to/new/db");
receiver.receive_sst_files(files).await?;
```

## Security

### Row-Level Security (RLS)

REL expressions evaluated per-node:

```rust
use raisin_rocksdb::security::ConditionEvaluator;

let evaluator = ConditionEvaluator::new(&auth_context);
let allowed = evaluator.evaluate(
    "node.created_by == auth.user_id || auth.roles.contains('admin')",
    &node
)?;
```

### Identity Management

```rust
// Create identity with provider
storage.upsert_identity(&tenant, IdentityInput {
    provider: "github",
    provider_user_id: "12345",
    email: Some("user@example.com"),
    ..Default::default()
})?;

// Create session
let session = storage.create_session(&tenant, &identity_id, SessionOptions {
    ttl: Duration::from_secs(3600),
    ..Default::default()
})?;
```

## Background Jobs

Unified job queue for async operations:

```rust
use raisin_rocksdb::{JobDataStore, JobContext};

// Register job
let job_id = storage.enqueue_job(JobContext {
    job_type: "embedding_generation",
    payload: serde_json::json!({ "node_id": "..." }),
    ..Default::default()
})?;

// Process jobs (via JobRegistry)
let jobs = storage.get_pending_jobs("embedding_generation", 10)?;
```

## Components

| Module | Description |
|--------|-------------|
| `storage` | `RocksDBStorage` central abstraction |
| `transaction` | `RocksDBTransaction` ACID transactions |
| `repositories/` | 25+ specialized data repositories |
| `replication/` | Operation capture, queue, applicator |
| `checkpoint/` | Checkpoint create/receive for sync |
| `security/` | RLS condition evaluation |
| `jobs/` | Background job queue and handlers |
| `graph/` | Graph traversal and algorithms |
| `spatial/` | Geospatial indexing |
| `lazy_indexing/` | Deferred index builds |
| `keys/` | Key encoding utilities |

## Benchmarks

Run benchmarks:

```bash
# Full benchmark suite
cargo bench -p raisin-rocksdb

# Specific benchmarks
cargo bench -p raisin-rocksdb rocksdb_benchmarks
cargo bench -p raisin-rocksdb persistent_idempotency_bench
```

## Testing

```bash
# Unit tests
cargo test -p raisin-rocksdb

# Integration tests
cargo test -p raisin-rocksdb --test '*'
```

## Documentation

Additional documentation in [`docs/`](./docs/):

- [MVCC Usage Examples](./docs/MVCC_USAGE_EXAMPLES.md) - Time-travel queries, audit logs, diffs, rollbacks
- [Async Operation Queue](./docs/ASYNC_OPERATION_QUEUE.md) - Replication queue architecture, config, monitoring

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
