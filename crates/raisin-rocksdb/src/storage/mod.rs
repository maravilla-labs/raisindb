//! Main RocksDB storage implementation
//!
//! This module provides the central storage abstraction for RaisinDB, coordinating
//! all persistence operations, repository access, background job processing, and
//! replication across the system.
//!
//! See the module-level [README.md](./README.md) for comprehensive documentation
//! on architecture, usage patterns, and configuration.

mod accessors;
mod deltas;
mod init;
mod jobs;
mod replication;
mod types;

pub use types::RestoreStats;

use crate::config::RocksDBConfig;
use crate::jobs::JobDataStore;
use crate::lazy_indexing::LazyIndexManager;
use crate::repositories::*;
use crate::transaction::RocksDBTransaction;
use raisin_error::Result;
use raisin_events::EventBus;
use raisin_models::nodes::Node;
use raisin_models::workspace::DeltaOp;
use raisin_storage::jobs::JobRegistry;
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::Storage;
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB-backed storage implementation
///
/// This is the main storage struct that provides access to all repositories,
/// job system components, replication infrastructure, and transactional operations.
///
/// # Architecture
///
/// The storage instance coordinates:
/// - **Repository Layer**: All domain repositories (nodes, branches, workspaces, etc.)
/// - **Job System**: Background job processing with crash recovery
/// - **Replication**: Operation capture and peer synchronization
/// - **Event System**: In-memory event bus for reactive operations
///
/// # Example
///
/// ```rust,no_run
/// use raisin_rocksdb::{RocksDBStorage, RocksDBConfig};
/// use std::sync::Arc;
///
/// # async fn example() -> raisin_error::Result<()> {
/// // Create storage with production configuration
/// let config = RocksDBConfig::production()
///     .with_path("/var/lib/raisindb")
///     .with_background_jobs_enabled(true);
///
/// let storage = Arc::new(RocksDBStorage::with_config(config)?);
///
/// // Initialize job system if enabled
/// if storage.config().background_jobs_enabled {
///     // ... initialize engines and call init_job_system()
/// }
///
/// // Use storage for operations
/// let mut tx = storage.begin().await?;
/// // ... perform operations
/// tx.commit().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct RocksDBStorage {
    pub(crate) db: Arc<DB>,
    pub(crate) event_bus: Arc<dyn EventBus>,
    pub(crate) config: RocksDBConfig,

    // Repository implementations
    pub(crate) nodes: NodeRepositoryImpl,
    pub(crate) node_types: NodeTypeRepositoryImpl,
    pub(crate) archetypes: ArchetypeRepositoryImpl,
    pub(crate) element_types: ElementTypeRepositoryImpl,
    pub(crate) workspaces: WorkspaceRepositoryImpl,
    pub(crate) registry: RegistryRepositoryImpl,
    pub(crate) property_index: PropertyIndexRepositoryImpl,
    pub(crate) reference_index: ReferenceIndexRepositoryImpl,
    pub(crate) versioning: VersioningRepositoryImpl,
    pub(crate) repository_management: RepositoryManagementRepositoryImpl,
    pub(crate) branches: BranchRepositoryImpl,
    pub(crate) tags: TagRepositoryImpl,
    pub(crate) revisions: RevisionRepositoryImpl,
    pub(crate) garbage_collection: GarbageCollectionRepositoryImpl,
    pub(crate) trees: TreeRepositoryImpl,
    pub(crate) relations: RelationRepositoryImpl,
    pub(crate) translations: RocksDBTranslationRepository,
    pub(crate) fulltext_job_store: RocksDbJobStore,
    pub(crate) spatial_index: SpatialIndexRepository,
    pub(crate) compound_index: CompoundIndexRepositoryImpl,

    // Lazy indexing
    pub(crate) lazy_index_manager: LazyIndexManager,

    // Unified job system components
    pub(crate) job_registry: Arc<JobRegistry>,
    pub(crate) job_data_store: Arc<JobDataStore>,
    pub(crate) job_metadata_store: Arc<crate::jobs::JobMetadataStore>,

    // Job dispatcher (set after init_job_system, used for queue stats)
    pub(crate) job_dispatcher:
        Arc<std::sync::RwLock<Option<Arc<crate::jobs::dispatcher::JobDispatcher>>>>,

    // Replication components
    pub(crate) operation_capture: Arc<crate::OperationCapture>,
    pub(crate) operation_queue: Option<Arc<crate::replication::OperationQueue>>,
    pub(crate) replication_coordinator:
        Arc<tokio::sync::RwLock<Option<Arc<raisin_replication::ReplicationCoordinator>>>>,
}

// Storage trait implementation - provides access to all repositories
impl Storage for RocksDBStorage {
    type Tx = RocksDBTransaction;
    type Nodes = NodeRepositoryImpl;
    type NodeTypes = NodeTypeRepositoryImpl;
    type Archetypes = ArchetypeRepositoryImpl;
    type ElementTypes = ElementTypeRepositoryImpl;
    type Workspaces = WorkspaceRepositoryImpl;
    type Registry = RegistryRepositoryImpl;
    type PropertyIndex = PropertyIndexRepositoryImpl;
    type ReferenceIndex = ReferenceIndexRepositoryImpl;
    type Versioning = VersioningRepositoryImpl;
    type RepositoryManagement = RepositoryManagementRepositoryImpl;
    type Branches = BranchRepositoryImpl;
    type Tags = TagRepositoryImpl;
    type Revisions = RevisionRepositoryImpl;
    type GarbageCollection = GarbageCollectionRepositoryImpl;
    type Trees = TreeRepositoryImpl;
    type Relations = RelationRepositoryImpl;
    type Translations = RocksDBTranslationRepository;
    type FullTextJobStore = RocksDbJobStore;
    type SpatialIndex = SpatialIndexRepository;
    type CompoundIndex = CompoundIndexRepositoryImpl;

    fn nodes(&self) -> &Self::Nodes {
        &self.nodes
    }

    fn node_types(&self) -> &Self::NodeTypes {
        &self.node_types
    }

    fn archetypes(&self) -> &Self::Archetypes {
        &self.archetypes
    }

    fn element_types(&self) -> &Self::ElementTypes {
        &self.element_types
    }

    fn workspaces(&self) -> &Self::Workspaces {
        &self.workspaces
    }

    fn registry(&self) -> &Self::Registry {
        &self.registry
    }

    fn property_index(&self) -> &Self::PropertyIndex {
        &self.property_index
    }

    fn reference_index(&self) -> &Self::ReferenceIndex {
        &self.reference_index
    }

    fn versioning(&self) -> &Self::Versioning {
        &self.versioning
    }

    fn repository_management(&self) -> &Self::RepositoryManagement {
        &self.repository_management
    }

    fn branches(&self) -> &Self::Branches {
        &self.branches
    }

    fn tags(&self) -> &Self::Tags {
        &self.tags
    }

    fn revisions(&self) -> &Self::Revisions {
        &self.revisions
    }

    fn garbage_collection(&self) -> &Self::GarbageCollection {
        &self.garbage_collection
    }

    fn trees(&self) -> &Self::Trees {
        &self.trees
    }

    fn relations(&self) -> &Self::Relations {
        &self.relations
    }

    fn translations(&self) -> &Self::Translations {
        &self.translations
    }

    fn fulltext_job_store(&self) -> &Self::FullTextJobStore {
        &self.fulltext_job_store
    }

    fn spatial_index(&self) -> &Self::SpatialIndex {
        &self.spatial_index
    }

    fn compound_index(&self) -> &Self::CompoundIndex {
        &self.compound_index
    }

    async fn begin(&self) -> Result<Self::Tx> {
        Ok(RocksDBTransaction::new(
            self.db.clone(),
            self.event_bus.clone(),
            Arc::new(self.revisions.clone()),
            Arc::new(self.branches.clone()),
            Arc::new(self.nodes.clone()),
            self.job_registry.clone(),
            self.job_data_store.clone(),
            self.operation_capture.clone(),
            self.operation_queue.clone(),
            self.replication_coordinator.clone(),
            Arc::new(self.clone()), // Storage reference for schema validation
        ))
    }

    fn event_bus(&self) -> Arc<dyn EventBus> {
        self.event_bus.clone()
    }

    async fn put_workspace_delta(&self, scope: StorageScope<'_>, node: &Node) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.put_workspace_delta(tenant_id, repo_id, branch, workspace, node)
            .await
    }

    async fn get_workspace_delta(
        &self,
        scope: StorageScope<'_>,
        path: &str,
    ) -> Result<Option<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_workspace_delta(tenant_id, repo_id, branch, workspace, path)
            .await
    }

    async fn get_workspace_delta_by_id(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> Result<Option<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_workspace_delta_by_id(tenant_id, repo_id, branch, workspace, node_id)
            .await
    }

    async fn list_workspace_deltas(&self, scope: StorageScope<'_>) -> Result<Vec<DeltaOp>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.list_workspace_deltas(tenant_id, repo_id, branch, workspace)
            .await
    }

    async fn clear_workspace_deltas(&self, scope: StorageScope<'_>) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.clear_workspace_deltas(tenant_id, repo_id, branch, workspace)
            .await
    }

    async fn delete_workspace_delta(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        path: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.delete_workspace_delta(tenant_id, repo_id, branch, workspace, node_id, path)
            .await
    }
}

// TransactionalStorage trait implementation - provides transactional context
#[async_trait::async_trait]
impl TransactionalStorage for RocksDBStorage {
    async fn begin_context(&self) -> Result<Box<dyn TransactionalContext>> {
        let tx = RocksDBTransaction::new(
            self.db.clone(),
            self.event_bus.clone(),
            Arc::new(self.revisions.clone()),
            Arc::new(self.branches.clone()),
            Arc::new(self.nodes.clone()),
            self.job_registry.clone(),
            self.job_data_store.clone(),
            self.operation_capture.clone(),
            self.operation_queue.clone(),
            self.replication_coordinator.clone(),
            Arc::new(self.clone()), // Storage reference for schema validation
        );
        Ok(Box::new(tx) as Box<dyn TransactionalContext>)
    }
}
