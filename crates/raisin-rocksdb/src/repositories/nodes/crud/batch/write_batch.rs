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
    ) -> Result<()> {
        // Get column family handles
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Convert Node to StorageNode (excludes path from blob)
        let storage_node = StorageNode::from_node(node, parent_id_override.map(|s| s.to_string()));

        // Serialize StorageNode with named fields (NOT Node - path is excluded)
        // Using to_vec_named ensures nested types like RaisinReference serialize with
        // field names (e.g., "raisin:ref", "raisin:workspace"), avoiding ambiguity
        // with plain string arrays during deserialization.
        let node_value = rmp_serde::to_vec_named(&storage_node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        // 1. Store node blob with versioned key
        let node_key =
            keys::node_key_versioned(tenant_id, repo_id, branch, workspace, &node.id, revision);
        batch.put_cf(cf_nodes, node_key, node_value);

        // 2. Index by path with versioned key (path -> node_id)
        let path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, &node.path, revision,
        );
        batch.put_cf(cf_path, path_key, node.id.as_bytes());

        // 3. Index node_path with versioned key (node_id -> path) for O(1) path materialization
        let node_path_key = keys::node_path_key_versioned(
            tenant_id, repo_id, branch, workspace, &node.id, revision,
        );
        batch.put_cf(cf_node_path, node_path_key, node.path.as_bytes());

        // 4. Add property indexes (delegates to indexing module)
        self.add_property_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 5. Add reference indexes (delegates to indexing module)
        self.add_reference_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 6. Add relation indexes (delegates to indexing module)
        self.add_relation_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 7. Add ORDERED_CHILDREN index entry (if order_label provided)
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
