use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::translations::{
    JsonPointer, LocaleCode, LocaleOverlay, TranslationHashRecord, TranslationMeta,
};
use raisin_storage::{
    fulltext::{FullTextIndexJob, FullTextJobStore},
    scope::StorageScope,
    transactional::{TransactionalContext, TransactionalStorage},
    translations::TranslationRepository,
    EventBus, InMemoryEventBus, Storage,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::gc::InMemoryGarbageCollector;
use crate::relations::InMemoryRelationRepo;
use crate::tags::InMemoryTagRepo;
use crate::{
    InMemoryArchetypeRepo, InMemoryBranchRepo, InMemoryCompoundIndexRepo, InMemoryElementTypeRepo,
    InMemoryNodeRepo, InMemoryNodeTypeRepo, InMemoryPropertyIndexRepo, InMemoryReferenceIndexRepo,
    InMemoryRegistryRepo, InMemoryRepositoryManagement, InMemoryRevisionRepo,
    InMemorySpatialIndexRepo, InMemoryTreeRepo, InMemoryTx, InMemoryVersioningRepo,
    InMemoryWorkspaceRepo,
};

#[derive(Clone, Default)]
pub struct NoopTranslationRepo;

#[async_trait]
impl TranslationRepository for NoopTranslationRepo {
    async fn get_translation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
        _revision: &raisin_hlc::HLC,
    ) -> Result<Option<LocaleOverlay>> {
        Ok(None)
    }

    async fn store_translation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
        _overlay: &LocaleOverlay,
        _meta: &TranslationMeta,
    ) -> Result<()> {
        Ok(())
    }

    async fn get_block_translation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _block_uuid: &str,
        _locale: &LocaleCode,
        _revision: &raisin_hlc::HLC,
    ) -> Result<Option<LocaleOverlay>> {
        Ok(None)
    }

    async fn store_block_translation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _block_uuid: &str,
        _locale: &LocaleCode,
        _overlay: &LocaleOverlay,
        _meta: &TranslationMeta,
    ) -> Result<()> {
        Ok(())
    }

    async fn list_translations_for_node(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _revision: &raisin_hlc::HLC,
    ) -> Result<Vec<LocaleCode>> {
        Ok(Vec::new())
    }

    async fn list_nodes_with_translation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _locale: &LocaleCode,
        _revision: &raisin_hlc::HLC,
    ) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn mark_blocks_orphaned(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _block_uuids: &[String],
        _revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        Ok(())
    }

    async fn get_translation_meta(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
    ) -> Result<Option<TranslationMeta>> {
        Ok(None)
    }

    async fn get_translations_batch(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_ids: &[String],
        _locale: &LocaleCode,
        _revision: &raisin_hlc::HLC,
    ) -> Result<HashMap<String, LocaleOverlay>> {
        Ok(HashMap::new())
    }

    async fn store_hash_record(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
        _pointer: &JsonPointer,
        _record: &TranslationHashRecord,
    ) -> Result<()> {
        Ok(())
    }

    async fn store_hash_records_batch(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
        _records: &std::collections::HashMap<JsonPointer, TranslationHashRecord>,
    ) -> Result<()> {
        Ok(())
    }

    async fn get_hash_records(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
    ) -> Result<std::collections::HashMap<JsonPointer, TranslationHashRecord>> {
        Ok(HashMap::new())
    }

    async fn delete_hash_records(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _locale: &LocaleCode,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct NoopFullTextJobStore;

impl FullTextJobStore for NoopFullTextJobStore {
    fn enqueue(&self, _job: &FullTextIndexJob) -> Result<()> {
        Ok(())
    }

    fn dequeue(&self, _count: usize) -> Result<Vec<FullTextIndexJob>> {
        Ok(Vec::new())
    }

    fn complete(&self, _job_ids: &[String]) -> Result<()> {
        Ok(())
    }

    fn fail(&self, _job_id: &str, _error: &str) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct InMemoryStorage {
    pub(crate) nodes: InMemoryNodeRepo,
    pub(crate) node_types: InMemoryNodeTypeRepo,
    pub(crate) archetypes: InMemoryArchetypeRepo,
    pub(crate) element_types: InMemoryElementTypeRepo,
    pub(crate) workspaces: InMemoryWorkspaceRepo,
    pub(crate) registry: InMemoryRegistryRepo,
    pub(crate) property_index: InMemoryPropertyIndexRepo,
    pub(crate) reference_index: InMemoryReferenceIndexRepo,
    pub(crate) relations: InMemoryRelationRepo,
    pub(crate) versioning: InMemoryVersioningRepo,
    pub(crate) repository_management: InMemoryRepositoryManagement,
    pub(crate) branches: InMemoryBranchRepo,
    pub(crate) tags: InMemoryTagRepo,
    pub(crate) revisions: InMemoryRevisionRepo,
    pub(crate) trees: InMemoryTreeRepo,
    pub(crate) gc: InMemoryGarbageCollector,
    pub(crate) event_bus: Arc<dyn EventBus>,
    pub(crate) translations: NoopTranslationRepo,
    pub(crate) fulltext_job_store: NoopFullTextJobStore,
    pub(crate) spatial_index: InMemorySpatialIndexRepo,
    pub(crate) compound_index: InMemoryCompoundIndexRepo,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        let property_index = Arc::new(InMemoryPropertyIndexRepo::new());
        let reference_index = Arc::new(InMemoryReferenceIndexRepo::new());
        let event_bus = Arc::new(InMemoryEventBus::new()) as Arc<dyn EventBus>;

        Self {
            nodes: InMemoryNodeRepo::with_indexes(property_index.clone(), reference_index.clone()),
            node_types: InMemoryNodeTypeRepo::new(),
            archetypes: InMemoryArchetypeRepo::new(),
            element_types: InMemoryElementTypeRepo::new(),
            workspaces: InMemoryWorkspaceRepo::new(event_bus.clone()),
            registry: InMemoryRegistryRepo::new(),
            property_index: (*property_index).clone(),
            reference_index: (*reference_index).clone(),
            relations: InMemoryRelationRepo::new(),
            versioning: InMemoryVersioningRepo::default(),
            repository_management: InMemoryRepositoryManagement::new(event_bus.clone()),
            branches: InMemoryBranchRepo::new(event_bus.clone()),
            tags: InMemoryTagRepo::new(event_bus.clone()),
            revisions: InMemoryRevisionRepo::default(),
            trees: InMemoryTreeRepo::new(),
            gc: InMemoryGarbageCollector::default(),
            event_bus,
            translations: NoopTranslationRepo,
            fulltext_job_store: NoopFullTextJobStore,
            spatial_index: InMemorySpatialIndexRepo,
            compound_index: InMemoryCompoundIndexRepo,
        }
    }
}

impl Storage for InMemoryStorage {
    type Tx = InMemoryTx;
    type Nodes = InMemoryNodeRepo;
    type NodeTypes = InMemoryNodeTypeRepo;
    type Archetypes = InMemoryArchetypeRepo;
    type ElementTypes = InMemoryElementTypeRepo;
    type Workspaces = InMemoryWorkspaceRepo;
    type Registry = InMemoryRegistryRepo;
    type PropertyIndex = InMemoryPropertyIndexRepo;
    type ReferenceIndex = InMemoryReferenceIndexRepo;
    type Relations = InMemoryRelationRepo;
    type Versioning = InMemoryVersioningRepo;
    type RepositoryManagement = InMemoryRepositoryManagement;
    type Branches = InMemoryBranchRepo;
    type Tags = InMemoryTagRepo;
    type Revisions = InMemoryRevisionRepo;
    type Trees = InMemoryTreeRepo;
    type GarbageCollection = InMemoryGarbageCollector;
    type Translations = NoopTranslationRepo;
    type FullTextJobStore = NoopFullTextJobStore;
    type SpatialIndex = InMemorySpatialIndexRepo;
    type CompoundIndex = InMemoryCompoundIndexRepo;

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
    fn relations(&self) -> &Self::Relations {
        &self.relations
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
    fn trees(&self) -> &Self::Trees {
        &self.trees
    }
    fn garbage_collection(&self) -> &Self::GarbageCollection {
        &self.gc
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
        Ok(InMemoryTx::new(self.nodes.nodes.clone()))
    }

    fn event_bus(&self) -> Arc<dyn EventBus> {
        self.event_bus.clone()
    }

    // Workspace delta operations - TODO: implement properly for testing
    async fn put_workspace_delta(
        &self,
        _scope: StorageScope<'_>,
        _node: &raisin_models::nodes::Node,
    ) -> Result<()> {
        // TODO: Implement in-memory workspace delta storage
        Ok(())
    }

    async fn get_workspace_delta(
        &self,
        _scope: StorageScope<'_>,
        _path: &str,
    ) -> Result<Option<raisin_models::nodes::Node>> {
        // TODO: Implement in-memory workspace delta storage
        Ok(None)
    }

    async fn get_workspace_delta_by_id(
        &self,
        _scope: StorageScope<'_>,
        _node_id: &str,
    ) -> Result<Option<raisin_models::nodes::Node>> {
        // TODO: Implement in-memory workspace delta storage
        Ok(None)
    }

    async fn list_workspace_deltas(
        &self,
        _scope: StorageScope<'_>,
    ) -> Result<Vec<raisin_models::workspace::DeltaOp>> {
        // TODO: Implement in-memory workspace delta storage
        Ok(Vec::new())
    }

    async fn clear_workspace_deltas(&self, _scope: StorageScope<'_>) -> Result<()> {
        // TODO: Implement in-memory workspace delta storage
        Ok(())
    }

    async fn delete_workspace_delta(
        &self,
        _scope: StorageScope<'_>,
        _node_id: &str,
        _path: &str,
    ) -> Result<()> {
        // TODO: Implement in-memory workspace delta storage
        Ok(())
    }
}

#[async_trait]
impl TransactionalStorage for InMemoryStorage {
    async fn begin_context(&self) -> Result<Box<dyn TransactionalContext>> {
        let tx = InMemoryTx::new(self.nodes.nodes.clone());
        Ok(Box::new(tx) as Box<dyn TransactionalContext>)
    }
}
