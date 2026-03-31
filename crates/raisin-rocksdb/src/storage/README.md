# RocksDB Storage Module

This module provides the main RocksDB-backed storage implementation for RaisinDB. It coordinates all persistence operations, repository access, background job processing, and replication across the system.

## Overview

`RocksDBStorage` is the central storage abstraction that:

- Manages the underlying RocksDB database connection
- Provides access to all repository implementations (nodes, branches, workspaces, etc.)
- Coordinates the unified background job system for indexing and maintenance
- Integrates replication capture and coordination for multi-node deployments
- Handles transactional operations and workspace deltas
- Manages configuration and initialization of all storage subsystems

This is the primary entry point for all storage operations in RaisinDB.

## Architecture

```
RocksDBStorage
├── Database Layer (RocksDB)
│   └── Column Families (nodes, branches, workspaces, jobs, etc.)
├── Repository Layer
│   ├── Nodes, Branches, Workspaces
│   ├── Schema (NodeTypes, Archetypes, ElementTypes)
│   ├── Versioning (Revisions, Tags)
│   └── Indexes (PropertyIndex, ReferenceIndex, FullText)
├── Job System
│   ├── JobRegistry (in-memory job tracking)
│   ├── JobDataStore (persistent job context)
│   ├── JobMetadataStore (persistent job metadata)
│   └── Worker Pool (background job execution)
├── Replication System
│   ├── OperationCapture (records operations)
│   ├── OperationQueue (async batching)
│   └── ReplicationCoordinator (peer synchronization)
└── Event System
    └── EventBus (in-memory event distribution)
```

## Module Structure

The storage module is organized into focused submodules:

- **`mod.rs`** - Main struct, public API, Storage trait implementation
- **`init.rs`** - Initialization logic and configuration setup
- **`jobs.rs`** - Background job system initialization and management
- **`replication.rs`** - Replication coordinator integration and state management
- **`deltas.rs`** - Workspace delta operations (put, get, list, clear, delete)
- **`accessors.rs`** - Repository and component accessor methods
- **`types.rs`** - Supporting types (RestoreStats, etc.)

## Quick Start

### Basic Initialization (Development)

```rust
use raisin_rocksdb::{RocksDBStorage, RocksDBConfig};
use std::sync::Arc;

// Create storage with default development configuration
let storage = RocksDBStorage::new("/tmp/raisindb-dev")?;

// Or use explicit configuration
let config = RocksDBConfig::development().with_path("/tmp/raisindb-dev");
let storage = RocksDBStorage::with_config(config)?;
```

### Production Initialization

```rust
use raisin_rocksdb::{RocksDBStorage, RocksDBConfig};
use std::sync::Arc;

// Create storage with production configuration
let config = RocksDBConfig::production()
    .with_path("/var/lib/raisindb")
    .with_cluster_node_id("node-1")
    .with_replication_enabled(true)
    .with_background_jobs_enabled(true)
    .with_worker_pool_size(4);

let storage = Arc::new(RocksDBStorage::with_config(config)?);

// Initialize background job system (if enabled)
if storage.config().background_jobs_enabled {
    let tantivy_engine = Arc::new(TantivyIndexingEngine::new(...));
    let hnsw_engine = Arc::new(HnswIndexingEngine::new(...));

    let (worker_pool, shutdown_token) = storage
        .clone()
        .init_job_system(tantivy_engine, hnsw_engine)
        .await?;

    // Store handles for graceful shutdown
}

// Wait for replication readiness (multi-node deployments)
while !storage.is_ready_for_requests().await {
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}
```

### Repository Access

```rust
// Access repositories through the Storage trait
let node = storage.nodes().get(tenant_id, repo_id, branch, workspace, node_id).await?;
let branch_info = storage.branches().get(tenant_id, repo_id, branch).await?;

// Access implementation-specific methods
let divergence = storage.branches_impl().calculate_divergence(
    tenant_id, repo_id, source_branch, target_branch
).await?;
```

### Transaction Usage

```rust
// Begin a transaction
let mut tx = storage.begin().await?;

// Perform operations within the transaction
tx.nodes().create(tenant_id, repo_id, branch, workspace, node).await?;
tx.branches().update_head(tenant_id, repo_id, branch, revision).await?;

// Commit atomically
tx.commit().await?;
```

## Configuration

The storage system is configured via `RocksDBConfig`. Key settings include:

### Performance Tuning

- **`write_buffer_size`** - MemTable size (default: 64MB, production: 256MB)
- **`max_write_buffer_number`** - Number of MemTables (default: 3, production: 6)
- **`target_file_size_base`** - SST file size (default: 64MB, production: 256MB)
- **`max_background_jobs`** - Compaction threads (default: 2, production: 8)
- **`cache_size`** - Block cache size (default: 512MB, production: 2GB)

### Job System Configuration

- **`background_jobs_enabled`** - Enable worker pool (default: false)
- **`worker_pool_size`** - Number of worker threads (default: 2, production: 4-8)

### Replication Configuration

- **`replication_enabled`** - Enable operation capture (default: false)
- **`async_operation_queue`** - Use async batching (default: false, production: true)
- **`operation_queue_capacity`** - Queue size (default: 10000)
- **`operation_queue_batch_size`** - Batch size (default: 100)
- **`operation_queue_batch_timeout_ms`** - Batch timeout (default: 10ms)
- **`cluster_node_id`** - Unique node identifier (required for replication)

### Operation Log Compaction

- **`oplog_compaction_min_age_secs`** - Minimum age before compaction (default: 3600s)
- **`oplog_merge_property_updates`** - Merge consecutive property updates (default: true)
- **`oplog_compaction_batch_size`** - Operations per compaction batch (default: 1000)

## Background Job System

The storage system includes a unified background job system for asynchronous processing:

### Job Types

- **FulltextIndex** - Build Tantivy full-text indexes
- **EmbeddingIndex** - Build HNSW vector indexes
- **PropertyIndexBuild** - Build on-demand property indexes
- **Snapshot** - Create RocksDB snapshots
- **ReplicationGC** - Clean up old replication metadata
- **ReplicationSync** - Pull operations from peers
- **OpLogCompaction** - Compact operation logs

### Job System Architecture

```
EventBus → UnifiedJobEventHandler → JobRegistry → JobDataStore
                                         ↓
                                    WorkerPool
                                         ↓
                                  JobHandlerRegistry
                                         ↓
                    [FulltextHandler, EmbeddingHandler, ...]
```

### Creating Jobs

Jobs are created through two methods:

1. **Event-driven (automatic)**: Events are emitted by operations and automatically queued as jobs
2. **Manual (explicit)**: Call `queue_property_index_build()` or similar methods

```rust
// Automatic: NodeCreated event → FulltextIndex job
storage.nodes().create(...).await?; // Emits event, job auto-queued

// Manual: Queue property index build
let job_id = storage.queue_property_index_build(
    tenant_id, repo_id, branch, workspace
).await?;

// Track job progress
let status = storage.job_registry().get_job(&job_id).await?;
```

### Job Lifecycle

1. **Scheduled** - Job created, waiting for worker
2. **Running** - Worker processing job
3. **Completed** - Job finished successfully
4. **Failed** - Job encountered error (will retry if configured)

### Crash Recovery

The job system automatically restores pending jobs after restart:

- **Scheduled jobs** are restored and will execute when workers are available
- **Running jobs** are reset to Scheduled (assumed crashed mid-execution)
- **Orphaned metadata** (jobs without context) is cleaned up

This happens automatically during `init_job_system()`.

### Job Cleanup

Completed/failed jobs are automatically cleaned up after 24 hours by the `JobCleanupTask`.

## Replication System

The storage system integrates with the replication layer to support multi-node deployments.

### Replication Architecture

```
Write Operation
      ↓
OperationCapture.capture()
      ↓
OperationQueue (async batching)
      ↓
Persist to OPERATION_LOG CF
      ↓
ReplicationCoordinator.push_to_all_peers()
      ↓
[Peer Node 1, Peer Node 2, ...]
```

### Enabling Replication

```rust
let config = RocksDBConfig::production()
    .with_replication_enabled(true)
    .with_async_operation_queue(true)  // Recommended for production
    .with_cluster_node_id("node-1");

let storage = Arc::new(RocksDBStorage::with_config(config)?);

// Restore replication state for each repository
storage.restore_replication_state(tenant_id, repo_id).await?;
```

### Setting Up Replication Coordinator

```rust
use raisin_replication::{ReplicationCoordinator, ClusterConfig};

let cluster_config = ClusterConfig {
    node_id: "node-1".to_string(),
    peers: vec![
        PeerConfig {
            node_id: "node-2".to_string(),
            address: "http://node2:8080".to_string(),
            push_enabled: true,
        },
    ],
};

let coordinator = Arc::new(ReplicationCoordinator::new(
    storage.clone(),
    cluster_config,
).await?);

// Register coordinator with storage
storage.set_replication_coordinator(coordinator.clone()).await;

// Start coordinator
coordinator.start().await?;
```

### Operation Capture

Every write operation (node create, branch update, etc.) is automatically captured by `OperationCapture`:

- **Synchronous mode** - Operations written directly to OPERATION_LOG CF
- **Async mode** (recommended) - Operations batched in memory queue, flushed periodically

The captured operations are then pushed to peers via the `ReplicationCoordinator`.

### Readiness Check

In multi-node deployments, check readiness before serving requests:

```rust
while !storage.is_ready_for_requests().await {
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}
println!("Node is ready to serve requests");
```

A node is ready when:
- Replication is disabled (single-node mode), OR
- Replication coordinator is configured

## Workspace Deltas

Workspace deltas track uncommitted changes in a workspace before they're committed to a branch.

### Delta Operations

```rust
// Put a modified node into workspace deltas
storage.put_workspace_delta(tenant_id, repo_id, branch, workspace, &node).await?;

// Get a specific delta by path
let delta_node = storage.get_workspace_delta(
    tenant_id, repo_id, branch, workspace, "/path/to/node"
).await?;

// List all deltas in a workspace
let deltas = storage.list_workspace_deltas(tenant_id, repo_id, branch, workspace).await?;
for delta_op in deltas {
    match delta_op {
        DeltaOp::Upsert(node) => println!("Modified: {}", node.path),
        DeltaOp::Delete { node_id, path } => println!("Deleted: {}", path),
    }
}

// Clear all deltas (e.g., after commit)
storage.clear_workspace_deltas(tenant_id, repo_id, branch, workspace).await?;
```

### Delta Storage

Deltas are stored in the `WORKSPACE_DELTAS` column family with keys:

```
<tenant_id>\0<repo_id>\0<branch>\0<workspace>\0delta\0<operation>\0<path>
```

Where `<operation>` is either:
- `put` - Node was created or modified
- `delete` - Node was deleted

## Common Patterns

### Initialize Storage for Testing

```rust
use raisin_rocksdb::RocksDBStorage;
use tempfile::TempDir;

let temp_dir = TempDir::new()?;
let storage = RocksDBStorage::new(temp_dir.path())?;
```

### Create a Node with Transaction

```rust
let mut tx = storage.begin().await?;

let node = Node {
    id: nanoid::nanoid!(),
    path: "/test/node".to_string(),
    node_type: "Document".to_string(),
    properties: Default::default(),
    created_at: Utc::now(),
    updated_at: Utc::now(),
};

tx.nodes().create(tenant_id, repo_id, branch, workspace, node).await?;
tx.commit().await?;
```

### Queue a Background Job

```rust
// Queue property index build
let job_id = storage.queue_property_index_build(
    tenant_id, repo_id, branch, workspace
).await?;

// Wait for completion
loop {
    if let Some(job) = storage.job_registry().get_job(&job_id).await? {
        match job.status {
            JobStatus::Completed => {
                println!("Job completed successfully");
                break;
            }
            JobStatus::Failed => {
                println!("Job failed: {:?}", job.error);
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}
```

### Access Multiple Repositories

```rust
// Storage trait provides access to all repositories
let node = storage.nodes().get(...).await?;
let branch = storage.branches().get(...).await?;
let workspace = storage.workspaces().get(...).await?;

// Implementation-specific methods require explicit access
let divergence = storage.branches_impl().calculate_divergence(...).await?;
let lazy_index = storage.lazy_index_manager();
```

### Run Format Migrations

```rust
// Run on server startup to migrate data formats
storage.run_format_migration().await?;
```

## Testing

### Unit Testing Without Background Jobs

```rust
#[tokio::test]
async fn test_node_operations() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();

    // No need to initialize job system for basic operations
    let mut tx = storage.begin().await.unwrap();
    // ... perform operations
    tx.commit().await.unwrap();
}
```

### Integration Testing With Background Jobs

```rust
#[tokio::test]
async fn test_fulltext_indexing() {
    let temp_dir = TempDir::new().unwrap();
    let config = RocksDBConfig::development()
        .with_path(temp_dir.path())
        .with_background_jobs_enabled(true);

    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());

    // Initialize job system
    let tantivy_engine = Arc::new(TantivyIndexingEngine::new(...));
    let hnsw_engine = Arc::new(HnswIndexingEngine::new(...));
    let (worker_pool, shutdown_token) = storage
        .clone()
        .init_job_system(tantivy_engine, hnsw_engine)
        .await
        .unwrap();

    // ... perform operations that trigger jobs

    // Graceful shutdown
    shutdown_token.cancel();
}
```

### Testing Replication

```rust
#[tokio::test]
async fn test_replication_capture() {
    let temp_dir = TempDir::new().unwrap();
    let config = RocksDBConfig::development()
        .with_path(temp_dir.path())
        .with_replication_enabled(true)
        .with_cluster_node_id("test-node");

    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());

    // Operations will be captured in OPERATION_LOG CF
    let mut tx = storage.begin().await.unwrap();
    tx.nodes().create(...).await.unwrap();
    tx.commit().await.unwrap();

    // Verify operation was captured
    let ops = storage.operation_capture().get_operations(...).await.unwrap();
    assert_eq!(ops.len(), 1);
}
```

## Troubleshooting

### Background Jobs Not Running

**Problem**: Jobs are created but never execute

**Solutions**:
1. Ensure `background_jobs_enabled = true` in config
2. Verify `init_job_system()` was called
3. Check worker pool size is > 0
4. Confirm `RAISIN_MASTER_KEY` environment variable is set

### Replication Not Working

**Problem**: Operations not replicating to peers

**Solutions**:
1. Verify `replication_enabled = true` in config
2. Ensure `set_replication_coordinator()` was called
3. Check cluster config has correct peer addresses
4. Confirm `restore_replication_state()` was called for each repository

### High Memory Usage

**Problem**: RocksDB consuming excessive memory

**Solutions**:
1. Reduce `cache_size` in config
2. Lower `write_buffer_size` and `max_write_buffer_number`
3. Enable `async_operation_queue` to batch writes
4. Tune `max_background_jobs` for your hardware

### Slow Write Performance

**Problem**: Write operations are slow

**Solutions**:
1. Increase `write_buffer_size` and `max_write_buffer_number`
2. Enable `async_operation_queue` for replication
3. Increase `max_background_jobs` for more compaction threads
4. Use SSDs for database storage
5. Disable replication for single-node deployments

### Job Failures After Restart

**Problem**: Jobs fail after server restart

**Solutions**:
1. Check job restoration logs during startup
2. Verify `JobDataStore` and `JobMetadataStore` are persisted correctly
3. Ensure `restore_pending_jobs()` completes successfully
4. Review job error messages in `JobRegistry`

## Performance Considerations

### Write Amplification

RocksDB uses LSM-tree architecture with write amplification:
- Writes go to MemTable (in-memory)
- MemTable flushes to L0 SST files
- Background compaction merges SST files across levels

**Tuning**:
- Larger `write_buffer_size` reduces flush frequency
- More `max_write_buffer_number` increases write throughput
- Larger `target_file_size_base` reduces compaction overhead

### Read Amplification

Reads may scan multiple SST files across levels:
- Block cache reduces disk reads
- Bloom filters reduce false positives
- Compaction reduces level count

**Tuning**:
- Larger `cache_size` improves read performance
- More `max_background_jobs` speeds up compaction
- Use prefix iterators for range queries

### Replication Overhead

Operation capture adds minimal overhead:
- Synchronous mode: 1-2% write latency increase
- Async mode (recommended): < 0.5% write latency increase

**Tuning**:
- Use `async_operation_queue` in production
- Increase `operation_queue_batch_size` for higher throughput
- Lower `operation_queue_batch_timeout_ms` for lower latency

## Migration Guide

### Upgrading from Single-Node to Multi-Node

1. Add replication config to existing node:

```rust
let config = RocksDBConfig::production()
    .with_path("/var/lib/raisindb")
    .with_replication_enabled(true)
    .with_async_operation_queue(true)
    .with_cluster_node_id("node-1");
```

2. Restore replication state:

```rust
storage.restore_replication_state(tenant_id, repo_id).await?;
```

3. Start replication coordinator:

```rust
let coordinator = Arc::new(ReplicationCoordinator::new(...).await?);
storage.set_replication_coordinator(coordinator.clone()).await;
coordinator.start().await?;
```

### Enabling Background Jobs

1. Add job config:

```rust
let config = RocksDBConfig::production()
    .with_background_jobs_enabled(true)
    .with_worker_pool_size(4);
```

2. Initialize job system after storage creation:

```rust
let storage = Arc::new(RocksDBStorage::with_config(config)?);
let (worker_pool, shutdown_token) = storage
    .clone()
    .init_job_system(tantivy_engine, hnsw_engine)
    .await?;
```

3. Jobs will automatically process existing data and new events.

## See Also

- **`crate::config::RocksDBConfig`** - Configuration options
- **`crate::transaction::RocksDBTransaction`** - Transaction implementation
- **`crate::jobs`** - Background job system
- **`crate::replication`** - Replication layer
- **`crate::repositories`** - Individual repository implementations
- **`raisin_storage::Storage`** - Storage trait definition
