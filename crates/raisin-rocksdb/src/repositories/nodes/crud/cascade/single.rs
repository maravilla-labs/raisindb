//! Single node deletion with revision
//!
//! This module contains functions for deleting a single node using a specific
//! revision. This is primarily used as a helper for cascade delete operations.

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Delete a single node with a specific revision (synchronous)
    ///
    /// This is a helper for cascade delete that deletes a single node by writing
    /// tombstones to all indexes at the specified revision. Unlike the main
    /// delete_impl, this function:
    /// - Takes a pre-allocated revision (no revision allocation)
    /// - Does not update branch HEAD (caller's responsibility)
    /// - Does not index the change (caller's responsibility)
    /// - Is synchronous (no async)
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node` - The node to delete
    /// * `revision` - The revision to use for tombstone markers
    ///
    /// # Returns
    /// * `Ok(())` if node was deleted successfully
    /// * `Err` if deletion failed
    ///
    /// # Usage
    /// This is typically called from cascade operations where multiple nodes
    /// are deleted with the same revision in a single batch.
    pub(in super::super::super) fn delete_node_with_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        revision: &HLC,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();

        // Get column family handles
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_compound = cf_handle(&self.db, cf::COMPOUND_INDEX)?;
        let cf_spatial = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // Add all tombstones to batch using shared logic
        self.add_node_tombstones_to_batch(
            &mut batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            revision,
            cf_nodes,
            cf_path,
            cf_property,
            cf_relation,
            cf_ordered,
            cf_node_path,
            cf_compound,
            cf_spatial,
        )?;

        // Atomic commit
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    /// Delete a node without cascade (fails if has children)
    ///
    /// This is used when cascade=false in DeleteNodeOptions. It checks for
    /// children and fails if any exist, preventing orphaned nodes.
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node_id` - The ID of the node to delete
    /// * `check_has_children` - Whether to check for children before deleting
    ///
    /// # Returns
    /// * `Ok(true)` if node was deleted
    /// * `Ok(false)` if node didn't exist
    /// * `Err(Error::Validation)` if node has children and check_has_children=true
    /// * `Err` if deletion failed
    pub(in super::super::super) async fn delete_without_cascade(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        check_has_children: bool,
    ) -> Result<bool> {
        use raisin_error::Error;

        // Check if node exists
        let node_exists = self
            .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
            .await?
            .is_some();

        if !node_exists {
            return Ok(false);
        }

        // Check for children if requested
        if check_has_children {
            let children = self
                .list_children_impl(
                    tenant_id, repo_id, branch, workspace, node_id, None, // HEAD revision
                )
                .await?;

            if !children.is_empty() {
                return Err(Error::Validation(format!(
                    "Cannot delete node '{}': it has {} children. Enable cascade to delete descendants.",
                    node_id,
                    children.len()
                )));
            }
        }

        // Delete the node itself using the standard delete_impl
        self.delete_impl(tenant_id, repo_id, branch, workspace, node_id)
            .await
    }
}
