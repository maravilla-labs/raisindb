//! In-memory implementation of RelationRepository
//!
//! This is a stub implementation that provides no-op relationship functionality
//! for the in-memory storage backend.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{FullRelation, RelationRef};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::RelationRepository;

#[derive(Clone, Default)]
pub struct InMemoryRelationRepo;

impl InMemoryRelationRepo {
    pub fn new() -> Self {
        Self
    }
}

impl RelationRepository for InMemoryRelationRepo {
    async fn add_relation(
        &self,
        _scope: StorageScope<'_>,
        _source_node_id: &str,
        _source_node_type: &str,
        _relation: RelationRef,
    ) -> Result<()> {
        // TODO: Implement in-memory relationship storage
        Ok(())
    }

    async fn remove_relation(
        &self,
        _scope: StorageScope<'_>,
        _source_node_id: &str,
        _target_workspace: &str,
        _target_node_id: &str,
    ) -> Result<bool> {
        // TODO: Implement in-memory relationship storage
        Ok(false)
    }

    async fn get_outgoing_relations(
        &self,
        _scope: StorageScope<'_>,
        _node_id: &str,
        _max_revision: Option<&HLC>,
    ) -> Result<Vec<RelationRef>> {
        // TODO: Implement in-memory relationship storage
        Ok(Vec::new())
    }

    async fn get_incoming_relations(
        &self,
        _scope: StorageScope<'_>,
        _node_id: &str,
        _max_revision: Option<&HLC>,
    ) -> Result<Vec<(String, String, RelationRef)>> {
        // TODO: Implement in-memory relationship storage
        Ok(Vec::new())
    }

    async fn get_relations_by_type(
        &self,
        _scope: StorageScope<'_>,
        _node_id: &str,
        _target_node_type: &str,
        _max_revision: Option<&HLC>,
    ) -> Result<Vec<RelationRef>> {
        // TODO: Implement in-memory relationship storage
        Ok(Vec::new())
    }

    async fn remove_all_relations_for_node(
        &self,
        _scope: StorageScope<'_>,
        _node_id: &str,
    ) -> Result<()> {
        // TODO: Implement in-memory relationship storage
        Ok(())
    }

    async fn scan_relations_global(
        &self,
        _scope: BranchScope<'_>,
        _relation_type_filter: Option<&str>,
        _max_revision: Option<&HLC>,
    ) -> Result<Vec<(String, String, String, String, FullRelation)>> {
        // TODO: Implement in-memory relationship storage
        Ok(Vec::new())
    }
}
