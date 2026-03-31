//! Tree move operation - moves a node and all its descendants
//!
//! Since nodes are stored as StorageNode (without path), moving a tree only
//! updates indexes - NO node blob rewrites needed. This is O(K) where K is
//! the number of index entries, vs O(N*blob_size) with embedded paths.

use super::super::super::helpers::TOMBSTONE;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_storage::{
    BranchRepository, BranchScope, NodeRepository, RevisionRepository, StorageScope,
};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Move node tree (node + all descendants) to a new location
    ///
    /// For each node in the tree:
    /// - Tombstone old PATH_INDEX
    /// - Write new PATH_INDEX
    /// - Write new NODE_PATH (node_id -> new path)
    ///
    /// Only for root node:
    /// - Update ORDERED_CHILDREN (parent changes)
    ///
    /// # Algorithm
    /// 1. Validate move operation
    /// 2. Scan all descendants using prefix scan
    /// 3. In ONE atomic WriteBatch: update all indexes
    /// 4. Update branch HEAD
    ///
    /// # Performance
    /// - ONE WriteBatch for entire tree (atomic)
    /// - ONE revision for all nodes
    /// - No blob writes - only index updates
    /// - Node IDs are preserved (unlike copy+delete)
    pub(in crate::repositories::nodes) async fn move_node_tree_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        new_path: &str,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<()> {
        tracing::info!(
            "move_node_tree: moving tree from id={} to new_path={} (optimized - index only)",
            id,
            new_path
        );

        // Get existing root node
        let root_node = self
            .get_impl(tenant_id, repo_id, branch, workspace, id, false)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        let old_root_path = root_node.path.clone();

        // Validation 1: Cannot move root node
        self.validate_not_root_node(&old_root_path)?;

        // Extract target parent and new name from new_path
        let (target_parent_path, new_name) = new_path
            .rsplit_once('/')
            .map(|(parent, name)| {
                let parent_path = if parent.is_empty() {
                    "/".to_string()
                } else {
                    parent.to_string()
                };
                (parent_path, name.to_string())
            })
            .unwrap_or_else(|| ("/".to_string(), new_path.to_string()));

        // Validation 2: Target parent must exist
        let target_parent_node = self
            .validate_parent_exists(tenant_id, repo_id, branch, workspace, &target_parent_path)
            .await?;

        // Validation 3: Check workspace allows this node type
        let is_root_target = target_parent_path == "/";
        self.validate_workspace_allows_node_type(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            &root_node.node_type,
            is_root_target,
        )
        .await?;

        // Validation 4: Check if root node type is allowed under target parent's schema
        self.validate_parent_allows_child(
            BranchScope::new(tenant_id, repo_id, branch),
            &target_parent_node.node_type,
            &root_node.node_type,
        )
        .await?;

        // Validation 5: No circular reference
        self.validate_no_circular_reference(&old_root_path, &target_parent_path)
            .await?;

        // Validation 6: Check for duplicate names in target location
        self.validate_unique_child_name(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &target_parent_node.id,
            &new_name,
        )
        .await?;

        tracing::info!(
            "move_node_tree: source_path={}, target_path={}",
            old_root_path,
            new_path
        );

        // Collect all descendants (includes root at depth 0)
        let descendants =
            self.scan_descendants_ordered_impl(tenant_id, repo_id, branch, workspace, id, None)?;

        tracing::info!(
            "move_node_tree: found {} nodes to move (index-only updates)",
            descendants.len()
        );

        // Allocate single revision for entire tree move
        let revision = self.revision_repo.allocate_revision();

        // Prepare atomic WriteBatch
        let mut batch = WriteBatch::default();

        // Get column family handles
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Track old parent for replication
        let mut old_parent_id: Option<String> = None;

        // Process root node's ORDERED_CHILDREN (parent changes)
        if let Some(old_parent_path) =
            old_root_path
                .rsplit_once('/')
                .map(|(p, _)| if p.is_empty() { "/" } else { p })
        {
            if let Some(old_parent_node) = self
                .get_by_path_impl(tenant_id, repo_id, branch, workspace, old_parent_path, None)
                .await?
            {
                old_parent_id = Some(old_parent_node.id.clone());

                // Tombstone old ordered children entry
                if let Some(old_label) = self.get_order_label_for_child(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &old_parent_node.id,
                    id,
                )? {
                    let old_ordered_key = keys::ordered_child_key_versioned(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &old_parent_node.id,
                        &old_label,
                        &revision,
                        id,
                    );
                    batch.put_cf(cf_ordered, old_ordered_key, TOMBSTONE);

                    // Invalidate cached metadata
                    let metadata_key = keys::last_child_metadata_key(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &old_parent_node.id,
                    );
                    batch.delete_cf(cf_ordered, metadata_key);
                }
            }
        }

        // Add root node to new parent's ORDERED_CHILDREN
        let mut new_position: Option<String> = None;
        let new_parent_id = target_parent_node.id.clone();

        let order_label = {
            let existing = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &new_parent_id,
                id,
            )?;
            if let Some(existing) = existing {
                existing
            } else {
                let last = self.get_last_order_label(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &new_parent_id,
                )?;
                if let Some(ref l) = last {
                    match crate::fractional_index::inc(l) {
                        Ok(label) => label,
                        Err(e) => {
                            tracing::warn!(
                                parent_id = %new_parent_id,
                                last_label = %l,
                                error = %e,
                                "Corrupt order label detected in rename, falling back to first()"
                            );
                            crate::fractional_index::first()
                        }
                    }
                } else {
                    crate::fractional_index::first()
                }
            }
        };
        new_position = Some(order_label.clone());

        let ordered_key = keys::ordered_child_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &new_parent_id,
            &order_label,
            &revision,
            id,
        );
        batch.put_cf(cf_ordered, ordered_key, new_name.as_bytes());

        // Update cached last-child metadata
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, &new_parent_id);
        batch.put_cf(cf_ordered, metadata_key, order_label.as_bytes());

        // Process all nodes (root + descendants): update PATH_INDEX and NODE_PATH
        let mut moved_node_ids = Vec::new();
        for (node, depth) in &descendants {
            moved_node_ids.push(node.id.clone());

            // Calculate new path for this node
            let node_new_path = if *depth == 0 {
                new_path.to_string()
            } else {
                let relative = node
                    .path
                    .strip_prefix(&format!("{}/", old_root_path))
                    .unwrap_or(&node.path);
                format!("{}/{}", new_path, relative)
            };

            // Tombstone old PATH_INDEX
            let old_path_key = keys::path_index_key_versioned(
                tenant_id, repo_id, branch, workspace, &node.path, &revision,
            );
            batch.put_cf(cf_path, old_path_key, TOMBSTONE);

            // Write new PATH_INDEX
            let new_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node_new_path,
                &revision,
            );
            batch.put_cf(cf_path, new_path_key, node.id.as_bytes());

            // Write new NODE_PATH (node_id -> new path)
            let node_path_key = keys::node_path_key_versioned(
                tenant_id, repo_id, branch, workspace, &node.id, &revision,
            );
            batch.put_cf(cf_node_path, node_path_key, node_new_path.as_bytes());
        }

        // Atomic commit
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic tree move failed: {}", e)))?;

        tracing::info!(
            "move_node_tree: wrote {} index updates atomically (no blob writes!)",
            moved_node_ids.len() * 3
        );

        // Update branch HEAD
        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        // Capture move operation for replication
        if self.operation_capture.is_enabled() {
            let actor = operation_meta
                .as_ref()
                .map(|m| m.actor.clone())
                .unwrap_or_else(|| "system".to_string());

            self.operation_capture
                .capture_move_node(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    branch.to_string(),
                    id.to_string(),
                    old_parent_id,
                    Some(new_parent_id),
                    new_position,
                    actor,
                )
                .await?;
        }

        // Index node changes for revision tracking
        for node_id in &moved_node_ids {
            self.revision_repo
                .index_node_change(tenant_id, repo_id, &revision, node_id)
                .await?;
        }

        // Store operation metadata if provided
        if let Some(op_meta) = operation_meta {
            let rev_meta = raisin_storage::RevisionMeta {
                revision,
                parent: op_meta.parent_revision,
                merge_parent: None,
                branch: branch.to_string(),
                timestamp: op_meta.timestamp,
                actor: op_meta.actor.clone(),
                message: op_meta.message.clone(),
                is_system: op_meta.is_system,
                changed_nodes: vec![],
                changed_node_types: Vec::new(),
                changed_archetypes: Vec::new(),
                changed_element_types: Vec::new(),
                operation: Some(op_meta),
            };

            self.revision_repo
                .store_revision_meta(tenant_id, repo_id, rev_meta)
                .await?;
        }

        tracing::info!(
            "move_node_tree: complete - {} nodes moved (IDs preserved, no blob writes)",
            moved_node_ids.len()
        );
        Ok(())
    }
}
