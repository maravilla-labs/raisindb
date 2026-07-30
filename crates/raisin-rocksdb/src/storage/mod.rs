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
mod tenant_wipe;
mod types;

pub use tenant_wipe::TenantWipeReport;
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
/// use raisin_storage::{Storage, Transaction};
/// use raisin_storage::transactional::TransactionalContext;
/// use std::sync::Arc;
///
/// # async fn example() -> raisin_error::Result<()> {
/// // Create storage with production configuration
/// let mut config = RocksDBConfig::production().with_path("/var/lib/raisindb");
/// config.background_jobs_enabled = true;
///
/// let storage = Arc::new(RocksDBStorage::with_config(config)?);
///
/// // Initialize job system if enabled
/// if storage.config().background_jobs_enabled {
///     // ... initialize engines and call init_job_system()
/// }
///
/// // Use storage for operations. Commit requires an auth context
/// // (authorship stamping); use AuthContext::system() for system work.
/// let tx = storage.begin().await?;
/// tx.set_auth_context(raisin_models::auth::AuthContext::system())?;
/// // ... perform operations
/// Transaction::commit(&tx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct RocksDBStorage {
    pub(crate) db: Arc<DB>,
    pub(crate) event_bus: Arc<dyn EventBus>,
    pub(crate) config: RocksDBConfig,

    // Shared in-memory RELATES reachability cache. A single instance is owned
    // here and handed to BOTH the background `RelatesCache` compute job (which
    // populates it + the durable GRAPH_CACHE column family) and the graph
    // resolver factory (which reads it during RLS evaluation). Sharing one Arc
    // is what lets the resolver see the job's precomputed reachability.
    pub(crate) graph_cache_layer: Arc<crate::graph::GraphCacheLayer>,

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

    // Shared per-(tenant,repo,branch) lock for Tantivy index access.
    // Worker handlers AND the admin rebuild path acquire the same
    // mutex so a rebuild blocks concurrent batch indexing instead of
    // racing against a half-deleted index directory.
    pub(crate) index_lock_manager: Arc<crate::jobs::IndexLockManager>,

    // Per-(tenant,repo,branch,kind) fulltext indexing failure counter.
    // Owned here so the HTTP `/fulltext/errors` endpoint can read the
    // same in-memory state the worker writes to, without going
    // through the handler.
    pub(crate) fulltext_error_counter: crate::jobs::handlers::FulltextErrorCounter,

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

    /// The RocksDB backend CAN report spatial index state, so the planner gets a real
    /// answer instead of the trait's fail-closed `None`.
    ///
    /// A fresh store is returned per call rather than a cached field because
    /// `SpatialStateStore` is a thin `Arc<DB>` + `Arc<RwLock<HashMap>>` pair; the cache
    /// it carries is per-instance, so callers that consult availability in a hot loop
    /// should hold on to the returned handle rather than re-fetching it.
    fn spatial_state(&self) -> Option<Arc<dyn raisin_storage::spatial::SpatialStateSource>> {
        Some(Arc::new(crate::spatial_state::SpatialStateStore::new(
            self.db.clone(),
        )))
    }

    /// The RocksDB backend can administer its spatial index: read the local state
    /// records, census the keys physically present, and queue a local rebuild.
    ///
    /// The enqueuer is always present here because `RocksDBStorage` always owns a
    /// `JobRegistry`; it stays `Option` in the admin store so a future backend
    /// without a job system reports the limitation rather than accepting a rebuild
    /// request and dropping it.
    fn spatial_admin(&self) -> Option<Arc<dyn raisin_storage::spatial_admin::SpatialIndexAdmin>> {
        let enqueuer = Arc::new(crate::storage::jobs::spatial::JobSystemEnqueuer::new(
            self.job_registry.clone(),
            self.job_data_store.clone(),
        ));
        Some(Arc::new(crate::spatial_state::SpatialAdminStore::new(
            self.db.clone(),
            Some(enqueuer),
        )))
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

    fn graph_resolver<'a>(
        &'a self,
        scope: raisin_storage::scope::BranchScope<'a>,
        revision: &'a raisin_hlc::HLC,
    ) -> Option<Box<dyn raisin_rel::eval::RelationResolver + 'a>> {
        // Always cache-backed: the resolver's durable GRAPH_CACHE lookup is
        // gated on having an in-memory layer, so we pass the shared layer + db
        // and let it fall back to BFS on a miss.
        Some(Box::new(crate::security::RocksDBGraphResolver::with_cache(
            &self.relations,
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            revision,
            self.db.clone(),
            self.graph_cache_layer.clone(),
        )))
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
