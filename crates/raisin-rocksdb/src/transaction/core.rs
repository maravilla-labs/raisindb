//! Core transaction implementation
//!
//! This module contains the `RocksDBTransaction` struct and its `Transaction` trait implementation.
//!
//! # Architecture
//!
//! The `RocksDBTransaction` provides:
//! - **Snapshot isolation**: Consistent reads at a specific point in time
//! - **Read-your-writes semantics**: Uncommitted changes are visible within the transaction
//! - **Conflict detection**: Tracks read/write sets for optimistic concurrency control
//! - **Lock-free HLC allocation**: Single timestamp for all operations in a transaction
//!
//! # Transaction Lifecycle
//!
//! 1. **Creation**: `RocksDBTransaction::new()` initializes all internal state
//! 2. **Operations**: TransactionalContext methods modify the write batch
//! 3. **Commit**: Atomic write of all changes with metadata and event emission
//! 4. **Rollback**: Drop the write batch, no changes applied
//!
//! # MVCC and HLC
//!
//! All operations in a transaction share a single HLC timestamp. This ensures:
//! - Atomic visibility of changes (all or nothing)
//! - Consistent ordering across nodes
//! - Simple conflict detection
//!
//! The HLC is allocated lazily on the first write operation to avoid unnecessary
//! allocations for read-only transactions.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_events::EventBus;
use raisin_hlc::HLC;
use raisin_storage::{RevisionRepository, Transaction};
use rocksdb::{WriteBatch, DB};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::change_types::{SharedChangedNodes, SharedChangedTranslations};
use super::metadata::{ConflictTracker, ReadCache, TransactionMetadata};
use crate::repositories::{BranchRepositoryImpl, NodeRepositoryImpl, RevisionRepositoryImpl};
use crate::RocksDBStorage;

/// RocksDB transaction implementation with advanced features:
/// - Snapshot isolation for consistent reads
/// - Read-your-writes semantics via in-memory cache
/// - Conflict detection via read/write set tracking
/// - Lock-free HLC allocation for versioned keys
///
/// # Example
///
/// ```ignore
/// let tx = RocksDBTransaction::new(db, event_bus, ...);
/// tx.set_branch("main")?;
/// tx.set_actor("user@example.com")?;
/// tx.set_message("Update node properties")?;
///
/// // Make changes
/// tx.put_node("draft", &node).await?;
///
/// // Commit atomically
/// tx.commit().await?;
/// ```
pub struct RocksDBTransaction {
    pub(super) db: Arc<DB>,
    pub(super) batch: Arc<Mutex<WriteBatch>>,
    pub(super) event_bus: Arc<dyn EventBus>,
    pub(super) metadata: Arc<Mutex<TransactionMetadata>>,
    pub(super) revision_repo: Arc<RevisionRepositoryImpl>,
    pub(super) branch_repo: Arc<BranchRepositoryImpl>,
    pub(super) node_repo: Arc<NodeRepositoryImpl>,

    // Week 3 enhancements
    /// Read-your-writes cache for uncommitted changes
    pub(super) read_cache: Arc<Mutex<ReadCache>>,
    /// Conflict detection tracker for optimistic concurrency control
    pub(super) conflict_tracker: Arc<Mutex<ConflictTracker>>,
    /// Track nodes changed during transaction for revision snapshot creation
    pub(super) changed_nodes: SharedChangedNodes,
    /// Track translations changed during transaction
    pub(super) changed_translations: SharedChangedTranslations,

    // Background job system for async snapshot creation
    /// Job registry for background jobs
    pub(super) job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    /// Job data store for job context
    pub(super) job_data_store: Arc<crate::jobs::JobDataStore>,

    // Replication support
    /// Operation capture for CRDT replication
    pub(super) operation_capture: Arc<crate::OperationCapture>,
    /// Detailed change tracker for granular operation capture
    pub(super) change_tracker: Arc<Mutex<crate::replication::ChangeTracker>>,
    /// Optional async operation queue for high-throughput replication
    pub(super) operation_queue: Option<Arc<crate::replication::OperationQueue>>,
    /// Collected operations for real-time replication push
    pub(super) captured_operations: Arc<Mutex<Vec<raisin_replication::Operation>>>,
    /// Replication coordinator for pushing operations to peers
    pub(super) replication_coordinator:
        Arc<tokio::sync::RwLock<Option<Arc<raisin_replication::ReplicationCoordinator>>>>,

    /// Storage reference for schema validation
    /// Used to create NodeValidator when schema validation is enabled
    pub(super) storage: Arc<RocksDBStorage>,

    /// Schema validation toggle (default: true)
    /// When true, node operations are validated against their NodeType/Archetype/ElementType schemas
    /// Can be disabled via TransactionalContext::set_validate_schema(false) for bulk imports
    pub(super) validate_schema: Arc<Mutex<bool>>,

    /// Owner token for CREATE path reservations held by this transaction.
    ///
    /// `add_node` reserves each created path in the shared registry on
    /// `NodeRepositoryImpl` BEFORE checking committed storage, closing the
    /// check-then-write race between concurrent transactions creating the same
    /// path. Reservations are released after a successful commit, on rollback,
    /// or on drop (abandoned transaction).
    pub(super) path_reservation_owner: u64,
    /// Reservation keys currently held by this transaction.
    ///
    /// A set, not a list: a batched write holds one reservation per created path
    /// AND per auto-created ancestor folder for the whole transaction, so a
    /// thousand-item batch accumulates thousands of keys. The dedup check below
    /// runs on every reservation, and a linear scan over that made it quadratic.
    pub(super) reserved_create_paths: Arc<Mutex<HashSet<String>>>,

    /// Secret-store versions minted by THIS transaction: name -> (version,
    /// hash of the plaintext that version holds).
    ///
    /// Bulk SQL DML can `put_node` the same node twice inside one transaction
    /// (an INSERT followed by an UPDATE of the same row, say), and the
    /// transaction's HLC is allocated once for all of it — so both writes land
    /// on ONE node revision, the second overwriting the first in the batch.
    /// Minting a second secret version for the second write would leave `@1` an
    /// orphan nothing resolves to, and would make the version counter track
    /// statement count rather than value changes.
    ///
    /// The hash is why the memo is safe to reuse: if the second write carries a
    /// DIFFERENT plaintext, reusing the first version would store the wrong
    /// value under the reference that survives. A hash mismatch mints a new
    /// version; only a byte-identical rewrite is deduplicated. The plaintext
    /// itself is deliberately not retained.
    ///
    /// Same scratch-state shape as `reserved_create_paths` above: owned by the
    /// transaction, dropped with it.
    pub(super) vaulted_secrets: VaultedSecrets,
}

/// Secret name -> (version minted in this transaction, hash of its plaintext).
type VaultedSecrets = Arc<Mutex<HashMap<String, (u64, [u8; 32])>>>;

impl RocksDBTransaction {
    /// Create a new RocksDB transaction
    ///
    /// # Arguments
    ///
    /// * `db` - RocksDB database handle
    /// * `event_bus` - Event bus for emitting node change events
    /// * `revision_repo` - Repository for HLC timestamp allocation
    /// * `branch_repo` - Repository for branch HEAD updates
    /// * `node_repo` - Repository for node validation and operations
    /// * `job_registry` - Registry for background snapshot jobs
    /// * `job_data_store` - Store for job execution context
    /// * `operation_capture` - Capture system for CRDT replication
    /// * `operation_queue` - Optional queue for async replication
    /// * `replication_coordinator` - Coordinator for pushing to replication peers
    /// * `storage` - Full storage reference for schema validation
    ///
    /// # Returns
    ///
    /// A new transaction ready for operations
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
        node_repo: Arc<NodeRepositoryImpl>,
        job_registry: Arc<raisin_storage::jobs::JobRegistry>,
        job_data_store: Arc<crate::jobs::JobDataStore>,
        operation_capture: Arc<crate::OperationCapture>,
        operation_queue: Option<Arc<crate::replication::OperationQueue>>,
        replication_coordinator: Arc<
            tokio::sync::RwLock<Option<Arc<raisin_replication::ReplicationCoordinator>>>,
        >,
        storage: Arc<RocksDBStorage>,
    ) -> Self {
        let path_reservation_owner = node_repo.new_path_reservation_owner();
        Self {
            db,
            batch: Arc::new(Mutex::new(WriteBatch::default())),
            event_bus,
            metadata: Arc::new(Mutex::new(TransactionMetadata::default())),
            revision_repo,
            branch_repo,
            node_repo,
            read_cache: Arc::new(Mutex::new(ReadCache::default())),
            conflict_tracker: Arc::new(Mutex::new(ConflictTracker::default())),
            changed_nodes: Arc::new(Mutex::new(HashMap::new())),
            changed_translations: Arc::new(Mutex::new(HashMap::new())),
            job_registry,
            job_data_store,
            operation_capture,
            change_tracker: Arc::new(Mutex::new(crate::replication::ChangeTracker::new())),
            operation_queue,
            captured_operations: Arc::new(Mutex::new(Vec::new())),
            replication_coordinator,
            storage,
            validate_schema: Arc::new(Mutex::new(true)), // Default: validation enabled
            path_reservation_owner,
            reserved_create_paths: Arc::new(Mutex::new(HashSet::new())),
            vaulted_secrets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The version this transaction already minted for `name` holding exactly
    /// this plaintext, if any.
    ///
    /// See [`RocksDBTransaction::vaulted_secrets`] for why a second write of the
    /// same node in one transaction reuses it rather than appending, and why a
    /// changed plaintext must not.
    pub(super) fn vaulted_secret_version(
        &self,
        name: &str,
        plaintext_hash: &[u8; 32],
    ) -> Option<u64> {
        self.vaulted_secrets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .filter(|(_, hash)| hash == plaintext_hash)
            .map(|(version, _)| *version)
    }

    /// Record the version minted for `name` by this transaction.
    pub(super) fn record_vaulted_secret(
        &self,
        name: String,
        version: u64,
        plaintext_hash: [u8; 32],
    ) {
        self.vaulted_secrets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name, (version, plaintext_hash));
    }

    /// Reserve a CREATE path for this transaction.
    ///
    /// Fails with `Conflict` if another in-flight creator (transaction or
    /// non-transactional create) already reserved the same scoped path.
    /// Must be called BEFORE the committed-storage existence check so that a
    /// concurrent winner's committed row is guaranteed visible to the loser.
    pub(super) fn reserve_create_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<()> {
        let key = self.node_repo.try_reserve_create_path(
            tenant_id,
            repo_id,
            branch,
            workspace,
            path,
            self.path_reservation_owner,
        )?;
        let mut reserved = self
            .reserved_create_paths
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        reserved.insert(key);
        Ok(())
    }

    /// Release every CREATE path reservation held by this transaction.
    ///
    /// Idempotent: keys are drained, and the registry ignores keys no longer
    /// owned by this transaction. Called after a successful commit, on
    /// rollback, and on drop.
    pub(super) fn release_create_path_reservations(&self) {
        let keys: Vec<String> = match self.reserved_create_paths.lock() {
            Ok(mut guard) => guard.drain().collect(),
            Err(poisoned) => poisoned.into_inner().drain().collect(),
        };
        if !keys.is_empty() {
            self.node_repo
                .release_create_path_reservations(self.path_reservation_owner, &keys);
        }
    }

    /// Get or allocate the single transaction HLC timestamp
    ///
    /// All operations in a transaction MUST share the same HLC to maintain atomicity.
    /// This method allocates an HLC on the first write operation and returns it for
    /// all subsequent operations in the transaction.
    ///
    /// # Lock-Free Allocation
    ///
    /// HLC allocation is lock-free at the RevisionRepository level, using atomic operations.
    /// We only need a lock here to coordinate the single allocation within this transaction.
    ///
    /// # Returns
    ///
    /// The transaction's HLC timestamp
    pub(super) fn get_or_allocate_transaction_revision(&self) -> Result<HLC> {
        // Check if we already have a transaction revision
        {
            let metadata = self
                .metadata
                .lock()
                .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
            if let Some(hlc) = metadata.transaction_revision {
                return Ok(hlc);
            }
        }

        // Allocate a single HLC for the entire transaction (lock-free at repo level)
        let hlc = self.revision_repo.allocate_revision();

        // Store it in metadata
        {
            let mut metadata = self
                .metadata
                .lock()
                .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
            metadata.transaction_revision = Some(hlc);
        }

        tracing::debug!("Allocated transaction HLC {:?}", hlc);

        Ok(hlc)
    }

    /// Check for conflicts with other transactions
    ///
    /// # Current Implementation
    ///
    /// This is a placeholder for future optimistic concurrency control.
    /// Currently tracks read/write sets but doesn't enforce conflicts.
    ///
    /// # Future Implementation
    ///
    /// A full implementation would:
    /// 1. Check if any keys in our read_set have been written by committed transactions
    /// 2. Check if any keys in our write_set have been read by other active transactions
    /// 3. Abort the transaction if conflicts are detected
    ///
    /// This would require a transaction manager to track all active transactions.
    pub(super) fn check_conflicts(&self) -> Result<()> {
        // Note: Full conflict detection would require a transaction manager
        // that tracks all active transactions. For now, we just track
        // the read/write sets for future implementation.

        // In a full implementation, this would:
        // 1. Check if any keys in our read_set have been written by committed transactions
        // 2. Check if any keys in our write_set have been read by other active transactions

        // For now, we return Ok as basic optimistic concurrency control
        Ok(())
    }

    /// Record a read operation for conflict detection
    ///
    /// # Arguments
    ///
    /// * `key` - The key that was read
    pub(super) fn record_read(&self, key: Vec<u8>) -> Result<()> {
        let mut tracker = self
            .conflict_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        tracker.read_set.insert(key);
        Ok(())
    }

    /// Record a write operation for conflict detection
    ///
    /// # Arguments
    ///
    /// * `key` - The key that was written
    pub(super) fn record_write(&self, key: Vec<u8>) -> Result<()> {
        let mut tracker = self
            .conflict_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        tracker.write_set.insert(key);
        Ok(())
    }

    /// Get a reference to the write batch (requires locking)
    ///
    /// # Returns
    ///
    /// An Arc to the mutex-protected write batch
    pub fn batch(&self) -> Arc<Mutex<WriteBatch>> {
        self.batch.clone()
    }

    /// Get the DB handle
    ///
    /// # Returns
    ///
    /// A reference to the RocksDB instance
    pub fn db(&self) -> &Arc<DB> {
        &self.db
    }

    /// Check if schema validation is enabled
    ///
    /// # Returns
    ///
    /// true if schema validation is enabled (default), false otherwise
    pub(super) fn is_validate_schema_enabled(&self) -> bool {
        *self
            .validate_schema
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Set schema validation toggle
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable schema validation
    pub(super) fn set_validate_schema_enabled(&self, enabled: bool) {
        *self
            .validate_schema
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = enabled;
    }

    /// Create a NodeValidator for schema validation
    ///
    /// Uses the transaction's storage and metadata to create a validator
    /// configured for the current tenant, repo, and branch.
    ///
    /// # Returns
    ///
    /// A NodeValidator instance ready to validate nodes
    pub(super) fn create_validator(
        &self,
    ) -> raisin_core::services::node_validation::NodeValidator<RocksDBStorage> {
        let metadata = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        let branch = metadata
            .branch
            .as_ref()
            .map(|b| b.to_string())
            .unwrap_or_default();
        raisin_core::services::node_validation::NodeValidator::new(
            self.storage.clone(),
            metadata.tenant_id.to_string(),
            metadata.repo_id.to_string(),
            branch,
        )
    }
}

#[async_trait]
impl Transaction for RocksDBTransaction {
    /// Commit the transaction atomically
    ///
    /// # Commit Phases
    ///
    /// The commit process consists of several phases:
    ///
    /// 1. **Conflict Check**: Verify no conflicts with other transactions
    /// 2. **Metadata Extraction**: Extract tenant, repo, branch, actor, message
    /// 3. **Data Collection**: Extract changed nodes and translations
    /// 4. **RevisionMeta Creation**: Create revision metadata for the commit
    /// 5. **Atomic Write**: Write everything to RocksDB in a single batch
    /// 6. **Replication**: Capture and push operations to peers
    /// 7. **Background Jobs**: Enqueue snapshot creation job
    /// 8. **Event Emission**: Emit NodeEvent for each changed node
    ///
    /// All database writes happen atomically in phase 5. If any phase fails,
    /// the entire transaction is rolled back.
    ///
    /// # Returns
    ///
    /// Ok(()) if commit succeeds, Error otherwise
    #[allow(clippy::manual_async_fn)]
    fn commit(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            // Delegate to commit module for implementation
            // This keeps the core module focused on structure and lifecycle
            super::commit::commit_impl(self).await
        }
    }

    /// Rollback the transaction
    ///
    /// Drops the write batch, discarding all changes. No data is written to RocksDB.
    ///
    /// # Returns
    ///
    /// Ok(()) - rollback always succeeds
    #[allow(clippy::manual_async_fn)]
    fn rollback(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async move {
            // WriteBatch is dropped, no changes are applied.
            // Free any CREATE path reservations so other creators can proceed.
            self.release_create_path_reservations();
            Ok(())
        }
    }
}

impl Drop for RocksDBTransaction {
    fn drop(&mut self) {
        // Abandoned transactions (never committed nor rolled back) must not
        // leak CREATE path reservations, or the paths would stay uncreatable
        // for the process lifetime. Releasing is idempotent.
        self.release_create_path_reservations();
    }
}
