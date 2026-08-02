//! Repository and component accessor methods

use super::RocksDBStorage;
use crate::jobs::JobDataStore;
use crate::lazy_indexing::LazyIndexManager;
use crate::repositories::{BranchRepositoryImpl, NodeRepositoryImpl};
use crate::repositories::{
    ProcessingRulesRepositoryImpl, RocksDBAuditRepo, TenantAIConfigRepository,
    TenantAuthConfigRepository, TenantEmbeddingConfigRepository,
};
use raisin_storage::jobs::JobRegistry;
use rocksdb::DB;
use std::sync::Arc;

impl RocksDBStorage {
    /// Get direct access to NodeRepositoryImpl (for testing)
    ///
    /// This allows tests to access implementation-specific methods like add()
    #[allow(dead_code)]
    pub fn nodes_impl(&self) -> &NodeRepositoryImpl {
        &self.nodes
    }

    /// Get direct access to BranchRepositoryImpl
    ///
    /// This allows access to implementation-specific methods like calculate_divergence()
    pub fn branches_impl(&self) -> &BranchRepositoryImpl {
        &self.branches
    }

    /// Get access to LazyIndexManager
    ///
    /// This allows access to lazy indexing operations for on-demand index building
    pub fn lazy_index_manager(&self) -> &LazyIndexManager {
        &self.lazy_index_manager
    }

    /// Get the shared in-memory RELATES reachability cache layer.
    ///
    /// This is the single instance the graph resolver reads from during RLS
    /// evaluation. Background graph-compute jobs must populate THIS instance
    /// (clone the Arc) so the resolver sees their precomputed reachability.
    pub fn graph_cache_layer(&self) -> Arc<crate::graph::GraphCacheLayer> {
        self.graph_cache_layer.clone()
    }

    /// Get the instance-based job registry
    ///
    /// Returns a reference to the JobRegistry used by this storage instance.
    /// This registry tracks all jobs created through the unified job system.
    ///
    /// # Returns
    ///
    /// Reference to the Arc-wrapped JobRegistry for this storage instance
    pub fn job_registry(&self) -> &Arc<JobRegistry> {
        &self.job_registry
    }

    /// Get the job metadata store
    ///
    /// Returns a reference to the JobMetadataStore for accessing persistent job data.
    ///
    /// # Returns
    ///
    /// Reference to the Arc-wrapped JobMetadataStore
    pub fn job_metadata_store(&self) -> &Arc<crate::jobs::JobMetadataStore> {
        &self.job_metadata_store
    }

    /// Get access to the JobDataStore
    ///
    /// # Returns
    ///
    /// Reference to the Arc-wrapped JobDataStore
    pub fn job_data_store(&self) -> &Arc<JobDataStore> {
        &self.job_data_store
    }

    /// Set the job dispatcher reference (called after init_job_system)
    pub fn set_job_dispatcher(&self, dispatcher: Arc<crate::jobs::dispatcher::JobDispatcher>) {
        let mut lock = self.job_dispatcher.write().unwrap();
        *lock = Some(dispatcher);
    }

    /// Get job dispatcher stats (queue lengths and dispatch counts)
    pub fn job_dispatcher_stats(&self) -> Option<crate::jobs::dispatcher::DispatcherStats> {
        let lock = self.job_dispatcher.read().unwrap();
        lock.as_ref().map(|d| d.stats())
    }

    /// Get the underlying RocksDB instance
    pub fn db(&self) -> &Arc<DB> {
        &self.db
    }

    /// Flush every column family and the WAL, then stop background work.
    ///
    /// Call this on the process's shutdown path, once serving has stopped.
    ///
    /// **Why it must be explicit.** The only thing that flushes on its own is
    /// the `DB` destructor, and `db` is an `Arc` cloned into services, job
    /// handlers and caches across the process — so returning from `main` does
    /// not reliably drop it, and even an orderly SIGTERM can leave the
    /// memtables unflushed and a large WAL behind. That WAL is then replayed
    /// on the *next* boot, which is what turns an unclean stop into a slow
    /// start.
    ///
    /// Blocking; call it from `spawn_blocking` (or outside the runtime), never
    /// on an async worker thread. Idempotent and safe to call while other
    /// `Arc<DB>` handles are still alive — it does not close the database.
    pub fn flush_and_close(&self) -> Result<(), rocksdb::Error> {
        // Memtables first: this is what makes the WAL replaceable.
        let mut first_error = None;
        for cf_name in crate::all_column_families() {
            if let Some(cf) = self.db.cf_handle(cf_name) {
                if let Err(e) = self.db.flush_cf(&cf) {
                    tracing::warn!(cf = %cf_name, error = %e, "Failed to flush column family on shutdown");
                    first_error.get_or_insert(e);
                }
            }
        }

        if let Err(e) = self.db.flush_wal(true) {
            tracing::warn!(error = %e, "Failed to sync the WAL on shutdown");
            first_error.get_or_insert(e);
        }

        // Stop compactions/flushes so nothing is mid-write when the process
        // exits. `false` = do not wait for unscheduled work to be scheduled.
        self.db.cancel_all_background_work(true);

        match first_error {
            Some(e) => Err(e),
            None => {
                tracing::info!("RocksDB flushed and background work cancelled");
                Ok(())
            }
        }
    }

    /// Shared per-(tenant,repo,branch) lock manager used by Tantivy
    /// indexing.
    ///
    /// Both batch-indexing worker handlers and the admin rebuild path
    /// must acquire the lock for a given key. Without this shared
    /// mutex, a rebuild could `remove_dir_all` while a worker is
    /// mid-commit, corrupting the index or panicking the worker.
    pub fn index_lock_manager(&self) -> &Arc<crate::jobs::IndexLockManager> {
        &self.index_lock_manager
    }

    /// Per-(tenant,repo,branch,kind) fulltext indexing failure
    /// counter. Worker handlers write to it on every error path; the
    /// HTTP layer reads via this accessor to render the admin
    /// console's "Index Errors" card.
    pub fn fulltext_error_counter(&self) -> &crate::jobs::handlers::FulltextErrorCounter {
        &self.fulltext_error_counter
    }

    /// Get the current configuration
    pub fn config(&self) -> &crate::config::RocksDBConfig {
        &self.config
    }

    /// Get the tenant embedding config repository
    ///
    /// This provides access to the persistent storage for tenant-level
    /// embedding configurations including encrypted API keys.
    pub fn tenant_embedding_config_repository(&self) -> TenantEmbeddingConfigRepository {
        TenantEmbeddingConfigRepository::new(self.db.clone())
    }

    /// Get the tenant AI config repository
    ///
    /// This provides access to the persistent storage for tenant-level
    /// unified AI/LLM configurations including multiple providers and encrypted API keys.
    pub fn tenant_ai_config_repository(&self) -> TenantAIConfigRepository {
        TenantAIConfigRepository::new(self.db.clone())
    }

    /// Get the tenant auth config repository
    ///
    /// This provides access to the persistent storage for tenant-level
    /// authentication configurations including provider settings and anonymous access.
    pub fn tenant_auth_config_repository(&self) -> TenantAuthConfigRepository {
        TenantAuthConfigRepository::new(self.db.clone())
    }

    /// Get the processing rules repository
    ///
    /// This provides access to the persistent storage for per-repository
    /// AI processing rules configuration (chunking, captioning, OCR settings, etc.)
    pub fn processing_rules_repository(&self) -> ProcessingRulesRepositoryImpl {
        ProcessingRulesRepositoryImpl::new(self.db.clone())
    }

    /// Get the persistent audit-log repository.
    ///
    /// Deliberately not part of the `Storage` trait: `raisin-storage` has no
    /// dependency on `raisin-audit`, and an associated type there would force
    /// every backend (including `raisin-storage-memory`) to grow one. Callers
    /// that need audit persistence construct it here, the same way the auth
    /// service and the ad-hoc config repositories are built.
    /// Each call mints a fresh repo over the same db handle; they share a
    /// process-wide write-sequence counter so two instances cannot mint the same
    /// key in the same millisecond.
    pub fn audit_repository(&self) -> RocksDBAuditRepo {
        RocksDBAuditRepo::new(self.db.clone())
    }

    /// Get access to the operation capture component
    ///
    /// This provides access to the underlying operation capture for advanced
    /// replication operations like manual operation replay or peer synchronization.
    pub fn operation_capture(&self) -> &Arc<crate::OperationCapture> {
        &self.operation_capture
    }
}
