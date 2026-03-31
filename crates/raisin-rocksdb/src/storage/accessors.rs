//! Repository and component accessor methods

use super::RocksDBStorage;
use crate::jobs::JobDataStore;
use crate::lazy_indexing::LazyIndexManager;
use crate::repositories::{BranchRepositoryImpl, NodeRepositoryImpl};
use crate::repositories::{
    ProcessingRulesRepositoryImpl, TenantAIConfigRepository, TenantAuthConfigRepository,
    TenantEmbeddingConfigRepository,
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

    /// Get access to the operation capture component
    ///
    /// This provides access to the underlying operation capture for advanced
    /// replication operations like manual operation replay or peer synchronization.
    pub fn operation_capture(&self) -> &Arc<crate::OperationCapture> {
        &self.operation_capture
    }
}
