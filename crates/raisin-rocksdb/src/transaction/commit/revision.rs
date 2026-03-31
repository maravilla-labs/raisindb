//! Revision metadata and branch HEAD update operations

use super::super::RocksDBTransaction;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::WriteBatch;

impl RocksDBTransaction {
    /// Create and serialize RevisionMeta, adding it to the batch
    pub(in crate::transaction) async fn create_revision_meta(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_revision: &HLC,
        actor: &str,
        message: &str,
        is_system: bool,
        changed_node_infos: Vec<raisin_storage::NodeChangeInfo>,
    ) -> Result<raisin_storage::RevisionMeta> {
        // Get current branch HEAD to use as parent revision
        let parent_revision = {
            let key = keys::branch_key(tenant_id, repo_id, branch_name);
            let cf_branches = cf_handle(&self.db, cf::BRANCHES)?;

            if let Ok(Some(bytes)) = self.db.get_cf(cf_branches, key) {
                if let Ok(branch) = rmp_serde::from_slice::<raisin_context::Branch>(&bytes) {
                    Some(branch.head)
                } else {
                    None
                }
            } else {
                None
            }
        };

        let revision_meta = raisin_storage::RevisionMeta {
            revision: *new_revision,
            parent: parent_revision,
            merge_parent: None,
            branch: branch_name.to_string(),
            timestamp: chrono::Utc::now(),
            actor: actor.to_string(),
            message: message.to_string(),
            is_system,
            changed_nodes: changed_node_infos,
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation: None,
        };

        tracing::debug!(
            "Adding RevisionMeta to batch: tenant={}, repo={}, revision={}, parent={:?}, branch={}, changed_nodes={}",
            tenant_id,
            repo_id,
            revision_meta.revision,
            revision_meta.parent,
            revision_meta.branch,
            revision_meta.changed_nodes.len()
        );

        // Serialize and add to batch
        let cf_revisions = cf_handle(&self.db, cf::REVISIONS)?;
        let meta_key = keys::revision_meta_key(tenant_id, repo_id, new_revision);
        let meta_value = rmp_serde::to_vec(&revision_meta).map_err(|e| {
            raisin_error::Error::storage(format!("RevisionMeta serialization error: {}", e))
        })?;
        batch.put_cf(cf_revisions, meta_key, meta_value);

        Ok(revision_meta)
    }

    /// Update branch HEAD in the batch and return branch update info
    pub(in crate::transaction) async fn update_branch_head(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: &HLC,
    ) -> Result<raisin_context::Branch> {
        tracing::debug!(
            "Adding branch HEAD update to batch: tenant={}, repo={}, branch={}, new_head={}",
            tenant_id,
            repo_id,
            branch_name,
            new_head
        );

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let cf_branches = cf_handle(&self.db, cf::BRANCHES)?;

        if let Ok(Some(bytes)) = self.db.get_cf(cf_branches, &key) {
            let mut branch: raisin_context::Branch =
                rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Branch deserialization error: {}", e))
                })?;

            branch.head = *new_head;

            let value = rmp_serde::to_vec(&branch).map_err(|e| {
                raisin_error::Error::storage(format!("Branch serialization error: {}", e))
            })?;

            batch.put_cf(cf_branches, key, value);

            Ok(branch)
        } else {
            Err(raisin_error::Error::NotFound(format!(
                "Branch '{}' not found during commit",
                branch_name
            )))
        }
    }
}
