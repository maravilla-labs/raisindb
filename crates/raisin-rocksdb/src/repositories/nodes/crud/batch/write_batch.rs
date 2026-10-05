//! WriteBatch node construction.
//!
//! Contains `add_node_to_batch` and `add_node_to_batch_with_parent_id` which
//! add a node to a WriteBatch with all necessary index entries (path, property,
//! reference, relation, ordered children).

use super::super::super::storage_node::StorageNode;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add a node to WriteBatch with all necessary index entries
    ///
    /// This is a helper for operations that need to manually construct WriteBatch
    /// entries (like copy_tree and delete_tree with single revision).
    ///
    /// # What it does
    /// - Adds node to NODES CF
    /// - Adds path index entry
    /// - Adds property indexes (including pseudo-properties)
    /// - Adds reference indexes (forward + reverse)
    /// - Adds relation indexes (forward + reverse)
    /// - Adds ORDERED_CHILDREN index entry (if order_label provided)
    ///
    /// # Parameters
    /// * `batch` - WriteBatch to add operations to
    /// * `node` - The node to add
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context
    /// * `revision` - The revision to use (IMPORTANT: caller controls this for atomicity)
    /// * `order_label` - Optional fractional index label for ORDERED_CHILDREN (None = skip ordering)
    ///
    /// # Returns
    /// Ok(()) if successful, Err if serialization or index construction fails
    pub(crate) fn add_node_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        order_label: Option<&str>,
    ) -> Result<()> {
        self.add_node_to_batch_with_parent_id(
            batch,
            node,
            tenant_id,
            repo_id,
            branch,
            workspace,
            revision,
            order_label,
            None,
            super::super::indexing::PropertyWrite::CREATE,
        )
    }

    pub(crate) fn add_node_to_batch_with_parent_id(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        order_label: Option<&str>,
        parent_id_override: Option<&str>,
        write: super::super::indexing::PropertyWrite<'_>,
    ) -> Result<()> {
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // 1 + 3. The node record — StorageNode blob (no path) and its
        // NODE_PATH entry — through the ONE record writer.
        super::super::indexing::node_record::write_node_record(
            &self.db,
            batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            parent_id_override.map(|s| s.to_string()),
            revision,
        )?;

        // 2. Index by path with versioned key (path -> node_id)
        let path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, &node.path, revision,
        );
        batch.put_cf(cf_path, path_key, node.id.as_bytes());

        // 4 + 5. Property and pseudo-property entries (__name, __node_type,
        // membership, ...) through the ONE writer — without the pseudo ones,
        // nodes created through this batch path (e.g. deep create) are
        // invisible to list_by_type.
        self.write_property_entries(
            batch, node, tenant_id, repo_id, branch, workspace, revision, write,
        )?;

        // 6. Add reference indexes (every reference, never skipped)
        self.add_reference_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 7. Add relation indexes (delegates to indexing module)
        self.add_relation_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 7b. Spatial indexes, through the ONE spatial writer every other node
        // writer calls. Copy, cross-branch promotion and deep create stage
        // nodes here, and without it a copied geometry was stored, reported a
        // healthy index, and was invisible to every ST_DWITHIN on the copy.
        // Synchronous, policy from the local state record — no schema read.
        self.add_spatial_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 8. Add ORDERED_CHILDREN index entry (if order_label provided)
        if let Some(label) = order_label {
            // Use parent_id_override if provided (for copy operations where node.parent is a NAME not ID)
            // Otherwise use node.parent (which should be an ID for regular operations)
            let parent_id = parent_id_override.or(node.parent.as_deref());
            if let Some(pid) = parent_id {
                let ordered_key = keys::ordered_child_key_versioned(
                    tenant_id, repo_id, branch, workspace, pid, label, revision, &node.id,
                );
                batch.put_cf(cf_ordered, ordered_key, node.name.as_bytes());
            }
        }

        Ok(())
    }
}
