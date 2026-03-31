//! Transaction implementation for RocksDB with MVCC support
//!
//! This module provides a complete transaction system for RocksDB with:
//! - **Snapshot isolation**: Consistent reads at a specific point in time
//! - **Read-your-writes semantics**: Uncommitted changes are visible within the transaction
//! - **Conflict detection**: Optimistic concurrency control with read/write set tracking
//! - **Lock-free HLC allocation**: Single timestamp for all operations in a transaction
//! - **CRDT replication support**: Operation capture for distributed synchronization
//!
//! # Architecture
//!
//! The transaction module is organized into several specialized submodules:
//!
//! ## Core Modules
//!
//! - **`core`**: Core `RocksDBTransaction` struct and `Transaction` trait implementation
//!   - Transaction lifecycle management
//!   - HLC allocation and conflict tracking
//!   - Commit/rollback coordination
//!
//! - **`context`**: `TransactionalContext` trait implementation
//!   - All CRUD operations for nodes, translations, and workspaces
//!   - Read-your-writes cache management
//!   - Index maintenance (PATH, PROPERTY, REFERENCE, ORDERED_CHILDREN)
//!
//! ## Supporting Modules
//!
//! - **`commit`**: Commit phase logic
//!   - Metadata extraction and batch handling
//!   - RevisionMeta creation
//!   - Branch HEAD updates
//!   - Snapshot job enqueueing
//!   - Event emission
//!   - Replication peer synchronization
//!
//! - **`metadata`**: Transaction state management
//!   - TransactionMetadata: tenant, repo, branch, actor, message, HLC
//!   - ReadCache: In-memory cache for read-your-writes semantics
//!   - ConflictTracker: Read/write set tracking for optimistic concurrency
//!
//! - **`types`**: Helper types and utilities
//!   - Tombstone detection
//!   - Reference extraction from properties
//!   - Pure utility functions
//!
//! - **`replication`**: CRDT replication support
//!   - Operation capture for distributed synchronization
//!   - Change tracking for granular operations
//!   - Peer push coordination
//!
//! # Transaction Lifecycle
//!
//! ```ignore
//! // 1. Create transaction
//! let tx = storage.transaction().await?;
//!
//! // 2. Set metadata
//! tx.set_branch("main")?;
//! tx.set_actor("user@example.com")?;
//! tx.set_message("Update node properties")?;
//!
//! // 3. Make changes
//! tx.put_node("draft", &node).await?;
//! tx.store_translation("draft", &node.id, "en", overlay).await?;
//!
//! // 4. Commit atomically
//! tx.commit().await?;
//! ```
//!
//! # MVCC and Versioned Keys
//!
//! All data is stored with versioned keys containing HLC timestamps:
//! - Keys: `{tenant}\0{repo}\0{branch}\0{workspace}\0{entity}\0{id}\0{~revision}`
//! - Revisions are encoded in descending order for prefix iteration
//! - Tombstone markers (b"T") mark deleted entries
//!
//! This enables:
//! - Time-travel queries: Read data as it existed at any point in history
//! - Concurrent transactions: Multiple transactions can proceed without blocking
//! - Conflict detection: Read/write sets enable optimistic concurrency control
//!
//! # HLC Allocation
//!
//! All operations in a transaction share a single HLC timestamp:
//! - Ensures atomic visibility (all or nothing)
//! - Maintains consistent ordering across nodes
//! - Simplifies conflict detection
//!
//! The HLC is allocated lazily on the first write operation using lock-free
//! atomic operations at the RevisionRepository level.
//!
//! # Read-Your-Writes Semantics
//!
//! The transaction maintains an in-memory cache of uncommitted changes:
//! - Nodes: (workspace, node_id) -> Node
//! - Paths: (workspace, path) -> node_id
//! - Translations: (workspace, node_id, locale) -> LocaleOverlay
//!
//! All read operations check this cache first, ensuring that changes made
//! earlier in the transaction are visible to later operations.
//!
//! # Commit Process
//!
//! The commit process consists of several phases:
//!
//! 1. **Conflict Check**: Verify no conflicts with other transactions
//! 2. **Metadata Extraction**: Extract tenant, repo, branch, actor, message
//! 3. **Data Collection**: Extract changed nodes and translations
//! 4. **RevisionMeta Creation**: Create revision metadata for the commit
//! 5. **Atomic Write**: Write everything to RocksDB in a single batch
//! 6. **Replication**: Capture and push operations to peers
//! 7. **Background Jobs**: Enqueue snapshot creation job
//! 8. **Event Emission**: Emit NodeEvent for each changed node
//!
//! All database writes happen atomically in phase 5. If any phase fails,
//! the entire transaction is rolled back.
//!
//! # Replication Support
//!
//! The transaction system integrates with the CRDT replication layer:
//! - ChangeTracker: Captures granular property changes, moves, deletions
//! - OperationCapture: Converts changes to replication operations
//! - OperationQueue: Optional async queue for high-throughput replication
//! - ReplicationCoordinator: Pushes operations to peers
//!
//! # Performance Considerations
//!
//! - **Lock Scoping**: All lock acquisitions are carefully scoped to minimize hold times
//! - **Lock-Free HLC**: Revision allocation uses atomic operations, no global lock
//! - **Async-Aware**: Locks are released before async operations to prevent deadlocks
//! - **Batch Operations**: All writes use RocksDB WriteBatch for atomicity
//!
//! # Example: Bulk Import
//!
//! ```ignore
//! let tx = storage.transaction().await?;
//! tx.set_branch("main")?;
//! tx.set_actor("import@system")?;
//! tx.set_message("Bulk import 1000 nodes")?;
//!
//! for node in nodes {
//!     // add_node is optimized for new nodes (no existence check)
//!     tx.add_node("draft", &node).await?;
//! }
//!
//! // All 1000 nodes share the same HLC, committed atomically
//! tx.commit().await?;
//! ```

// Module declarations
pub(crate) mod change_types;
mod commit;
mod context;
mod core;
mod metadata;
mod replication;
mod types;

// Re-export the main transaction type
pub use core::RocksDBTransaction;

// Internal exports for use within the transaction module
