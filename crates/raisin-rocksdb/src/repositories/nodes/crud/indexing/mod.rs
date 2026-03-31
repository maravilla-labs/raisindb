//! Shared indexing operations for nodes
//!
//! This module contains reusable functions for building index entries that are shared
//! between create and update operations, following DRY principles.

mod compound_indexes;
mod property_indexes;
mod reference_indexes;
mod relation_indexes;
mod unique_indexes;

use super::super::storage_node::StorageNode;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add all index entries for a node to a WriteBatch
    ///
    /// This is the main entry point for indexing a node. It:
    /// - Stores the node blob (as StorageNode - path is NOT in the blob)
    /// - Adds path index (path -> node_id)
    /// - Adds node_path index (node_id -> path) for O(1) path materialization
    /// - Adds all property indexes (regular + system properties)
    /// - Adds reference indexes (forward + reverse)
    /// - Adds relation indexes (forward + reverse)
    ///
    /// Note: This does NOT add ORDERED_CHILDREN index - use add_ordered_children_to_batch for that
    ///
    /// # StorageNode Optimization
    ///
    /// The node blob is stored as `StorageNode` which excludes the `path` field.
    /// This enables O(1) move operations since only the root node blob needs updating,
    /// while descendant blobs remain unchanged (only path indexes are updated).
    pub(crate) fn add_node_indexes_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        self.add_node_indexes_to_batch_with_parent_id(
            batch, node, tenant_id, repo_id, branch, workspace, revision, None,
        )
    }

    /// Add all index entries for a node with an optional parent_id
    ///
    /// This variant allows passing a known parent_id to avoid lookups.
    /// The parent_id is stored in the StorageNode blob for efficient parent resolution.
    pub(crate) fn add_node_indexes_to_batch_with_parent_id(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        parent_id: Option<String>,
    ) -> Result<()> {
        // Get column family handles
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;

        // Convert Node to StorageNode (excludes path from blob)
        let storage_node = StorageNode::from_node(node, parent_id);

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

        // 4. Add property indexes
        self.add_property_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 5. Add system property indexes
        self.add_system_property_indexes(
            batch, node, tenant_id, repo_id, branch, workspace, revision,
        )?;

        // 6. Add reference indexes
        self.add_reference_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 7. Add relation indexes
        self.add_relation_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        Ok(())
    }
}
