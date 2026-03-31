//! Storage initialization and configuration
//!
//! This module handles the creation and initialization of RocksDBStorage instances,
//! including repository setup, job system components, and replication configuration.

use super::RocksDBStorage;
use crate::config::RocksDBConfig;
use crate::jobs::{JobDataStore, JobMetadataStore};
use crate::lazy_indexing::LazyIndexManager;
use crate::repositories::*;
use raisin_error::Result;
use raisin_events::InMemoryEventBus;
use raisin_storage::jobs::JobRegistry;
use rocksdb::DB;
use std::path::Path;
use std::sync::Arc;

impl RocksDBStorage {
    /// Create a new RocksDB storage instance with default development configuration
    ///
    /// This is a convenience method that uses `RocksDBConfig::development()`.
    /// For production deployments, use `with_config()` with an appropriate preset.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let config = RocksDBConfig::development().with_path(path.as_ref());
        Self::with_config(config)
    }

    /// Create a new RocksDB storage instance with a specific configuration
    ///
    /// This is the recommended way to create storage instances, as it allows
    /// full control over performance tuning and operational features.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use raisin_rocksdb::{RocksDBStorage, RocksDBConfig};
    ///
    /// let config = RocksDBConfig::production().with_path("/var/lib/raisindb");
    /// let storage = RocksDBStorage::with_config(config)?;
    /// # Ok::<(), raisin_error::Error>(())
    /// ```
    pub fn with_config(config: RocksDBConfig) -> Result<Self> {
        let db = crate::open_db_with_config(&config)?;
        Self::from_db_and_config(Arc::new(db), config)
    }

    /// Create a RocksDB storage instance from an existing DB handle with default config
    ///
    /// For backward compatibility. New code should use `from_db_and_config()`.
    pub fn from_db(db: Arc<DB>) -> Result<Self> {
        Self::from_db_and_config(db, RocksDBConfig::default())
    }

    /// Create a RocksDB storage instance from an existing DB handle and config
    pub fn from_db_and_config(db: Arc<DB>, config: RocksDBConfig) -> Result<Self> {
        let event_bus = Arc::new(InMemoryEventBus::new());

        // Get or generate cluster node ID for HLC
        let node_id = config
            .cluster_node_id
            .clone()
            .unwrap_or_else(|| nanoid::nanoid!());

        // Create revision and branch repositories first (needed by nodes repository)
        let revision_repo_arc = Arc::new(RevisionRepositoryImpl::new(db.clone(), node_id));
        let branch_repo_arc = Arc::new(BranchRepositoryImpl::new(db.clone()));
        let workspaces = WorkspaceRepositoryImpl::new(db.clone(), event_bus.clone());
        let workspace_repo_arc = Arc::new(workspaces.clone());

        // Initialize unified job system components
        let job_metadata_store = Arc::new(JobMetadataStore::new(db.clone()));
        let job_registry = Arc::new(JobRegistry::new().with_persistence(
            job_metadata_store.clone() as Arc<dyn raisin_storage::jobs::JobPersistence>,
        ));
        let job_data_store = Arc::new(JobDataStore::new(db.clone()));

        // Initialize replication components
        // Cluster node ID can be configured or generated - for now use a simple default
        // In production this should come from config or be persisted
        let cluster_node_id = config
            .cluster_node_id
            .clone()
            .unwrap_or_else(|| format!("node-{}", nanoid::nanoid!(8)));

        let (operation_capture, operation_queue) = if config.replication_enabled {
            if config.async_operation_queue {
                // Create with async queue for high-throughput
                let (capture, queue) = crate::OperationCapture::new_with_queue(
                    db.clone(),
                    cluster_node_id.clone(),
                    config.operation_queue_capacity,
                    config.operation_queue_batch_size,
                    std::time::Duration::from_millis(config.operation_queue_batch_timeout_ms),
                );

                tracing::info!(
                    cluster_node_id = %cluster_node_id,
                    replication_enabled = true,
                    async_queue = true,
                    queue_capacity = config.operation_queue_capacity,
                    batch_size = config.operation_queue_batch_size,
                    batch_timeout_ms = config.operation_queue_batch_timeout_ms,
                    "Initialized operation capture with async queue"
                );

                (capture, Some(queue))
            } else {
                // Create without queue (synchronous capture)
                let capture = Arc::new(crate::OperationCapture::new(
                    db.clone(),
                    cluster_node_id.clone(),
                ));

                tracing::info!(
                    cluster_node_id = %cluster_node_id,
                    replication_enabled = true,
                    async_queue = false,
                    "Initialized operation capture without async queue"
                );

                (capture, None)
            }
        } else {
            // Replication disabled
            let capture = Arc::new(crate::OperationCapture::disabled(db.clone()));

            tracing::info!(replication_enabled = false, "Operation capture disabled");

            (capture, None)
        };

        // Recreate branch repository with operation capture and job system
        let branch_repo_arc = Arc::new(
            BranchRepositoryImpl::new_with_capture(db.clone(), operation_capture.clone())
                .with_job_system(job_registry.clone(), job_data_store.clone()),
        );

        // Create schema repositories with operation capture now that it's available
        let node_type_repo_arc = Arc::new(NodeTypeRepositoryImpl::new_with_capture(
            db.clone(),
            revision_repo_arc.clone(),
            branch_repo_arc.clone(),
            operation_capture.clone(),
        ));
        let archetype_repo_arc = Arc::new(ArchetypeRepositoryImpl::new_with_capture(
            db.clone(),
            revision_repo_arc.clone(),
            branch_repo_arc.clone(),
            operation_capture.clone(),
        ));
        let element_type_repo_arc = Arc::new(ElementTypeRepositoryImpl::new_with_capture(
            db.clone(),
            revision_repo_arc.clone(),
            branch_repo_arc.clone(),
            operation_capture.clone(),
        ));
        let tag_repo_arc = Arc::new(TagRepositoryImpl::new(db.clone()));

        // Create storage instance
        let storage = Self {
            nodes: NodeRepositoryImpl::new(
                db.clone(),
                event_bus.clone(),
                revision_repo_arc.clone(),
                branch_repo_arc.clone(),
                tag_repo_arc.clone(),
                node_type_repo_arc.clone(),
                workspace_repo_arc.clone(),
                operation_capture.clone(),
            ),
            node_types: (*node_type_repo_arc).clone(),
            archetypes: (*archetype_repo_arc).clone(),
            element_types: (*element_type_repo_arc).clone(),
            workspaces,
            registry: RegistryRepositoryImpl::new_with_capture(
                db.clone(),
                event_bus.clone(),
                operation_capture.clone(),
            ),
            property_index: PropertyIndexRepositoryImpl::new(db.clone()),
            reference_index: ReferenceIndexRepositoryImpl::new(db.clone()),
            versioning: VersioningRepositoryImpl::new(db.clone()),
            repository_management: RepositoryManagementRepositoryImpl::new_with_capture(
                db.clone(),
                event_bus.clone(),
                operation_capture.clone(),
            ),
            branches: (*branch_repo_arc).clone(),
            tags: (*tag_repo_arc).clone(),
            revisions: (*revision_repo_arc).clone(),
            garbage_collection: GarbageCollectionRepositoryImpl::new(db.clone()),
            trees: TreeRepositoryImpl::new(db.clone()),
            relations: RelationRepositoryImpl::new(db.clone(), branch_repo_arc.clone()),
            translations: RocksDBTranslationRepository::new_with_capture(
                db.clone(),
                operation_capture.clone(),
            ),
            fulltext_job_store: RocksDbJobStore::new(db.clone()),
            spatial_index: SpatialIndexRepository::new(db.clone()),
            compound_index: CompoundIndexRepositoryImpl::new(db.clone()),
            lazy_index_manager: LazyIndexManager::new(db.clone()),
            job_registry: job_registry.clone(),
            job_data_store: job_data_store.clone(),
            job_metadata_store: job_metadata_store.clone(),
            job_dispatcher: Arc::new(std::sync::RwLock::new(None)), // Set after init_job_system()
            operation_capture: operation_capture.clone(),
            operation_queue: operation_queue.clone(),
            replication_coordinator: Arc::new(tokio::sync::RwLock::new(None)), // Will be initialized in start_replication()
            db: db.clone(),
            event_bus: event_bus.clone(),
            config: config.clone(),
        };

        Ok(storage)
    }

    /// Run one-time format migration from JSON to MessagePack
    ///
    /// This should be called during server startup. It checks for a marker file
    /// and only runs the migration if it hasn't been completed yet.
    ///
    /// # Returns
    /// * `Ok(())` - Migration completed or already done
    /// * `Err(Error)` - Migration failed
    pub async fn run_format_migration(&self) -> Result<()> {
        let data_dir = self.config.path.as_path();

        // Run JSON → MessagePack migration
        crate::management::format_migration::run_migration(self.db.clone(), data_dir).await?;

        // Run relation schema migration (old 3/5-field → new 5/8-field format)
        crate::management::format_migration::run_relation_schema_migration(
            self.db.clone(),
            data_dir,
        )
        .await?;

        // Run job metadata migration (old 13-field → new 14-field format with next_retry_at)
        crate::management::format_migration::run_job_metadata_migration(self.db.clone(), data_dir)
            .await?;

        // Run job type serialization migration (JSON objects → strings)
        crate::management::format_migration::run_job_type_serialization_migration(
            self.db.clone(),
            data_dir,
        )
        .await?;

        // Run tenant embedding config migration (remove node_type_settings field)
        crate::management::format_migration::run_tenant_embedding_config_migration(
            self.db.clone(),
            data_dir,
        )
        .await?;

        // Run revision metadata migration (add operation field)
        crate::management::format_migration::run_revision_meta_migration(self.db.clone(), data_dir)
            .await?;

        // Run revision metadata migration v3 (fix variable-length arrays)
        crate::management::format_migration::run_revision_meta_migration_v3(
            self.db.clone(),
            data_dir,
        )
        .await?;

        Ok(())
    }
}
