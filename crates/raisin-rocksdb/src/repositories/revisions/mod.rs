//! Revision repository implementation

mod trait_impl;

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::{NodeHLCState, HLC};
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct RevisionRepositoryImpl {
    pub(super) db: Arc<DB>,
    /// In-memory HLC state for lock-free revision allocation
    pub(super) hlc_state: Arc<NodeHLCState>,
}

impl RevisionRepositoryImpl {
    pub fn new(db: Arc<DB>, node_id: String) -> Self {
        let hlc_state = Arc::new(NodeHLCState::new(node_id));
        Self { db, hlc_state }
    }

    /// Update HLC from a remote operation (during replication)
    pub fn update_hlc(&self, remote_hlc: &HLC) -> HLC {
        self.hlc_state.update(remote_hlc)
    }

    // ========================================================================
    // Batch-aware methods for atomic operations
    // ========================================================================

    /// Add node change index entry to a WriteBatch (for atomic operations)
    ///
    /// This method writes to the provided batch instead of directly to the DB,
    /// allowing the caller to include it in a larger atomic transaction.
    pub fn index_node_change_to_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_id: &str,
    ) -> Result<()> {
        let key = keys::node_change_key(tenant_id, repo_id, node_id, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        batch.put_cf(cf, key, b"");
        Ok(())
    }

    /// Add node type change index entry to a WriteBatch (for atomic operations)
    pub fn index_node_type_change_to_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_type_name: &str,
    ) -> Result<()> {
        let key = keys::node_type_change_key(tenant_id, repo_id, node_type_name, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        batch.put_cf(cf, key, b"");
        Ok(())
    }

    /// Add archetype change index entry to a WriteBatch (for atomic operations)
    pub fn index_archetype_change_to_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        archetype_name: &str,
    ) -> Result<()> {
        let key = keys::archetype_change_key(tenant_id, repo_id, archetype_name, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        batch.put_cf(cf, key, b"");
        Ok(())
    }

    /// Add element type change index entry to a WriteBatch (for atomic operations)
    pub fn index_element_type_change_to_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        element_type_name: &str,
    ) -> Result<()> {
        let key = keys::element_type_change_key(tenant_id, repo_id, element_type_name, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        batch.put_cf(cf, key, b"");
        Ok(())
    }
}
