//! Single node move operation
//!
//! Moves a single node to a new path by updating only indexes
//! (PATH_INDEX, NODE_PATH, ORDERED_CHILDREN). No blob rewrites needed.

use super::super::super::helpers::TOMBSTONE;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_storage::{
    BranchRepository, BranchScope, NodeRepository, RevisionRepository, StorageScope,
};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Move a single node to a new path (optimized for StorageNode)
    ///
    /// Since nodes are stored without path (StorageNode), moving only updates indexes:
    /// - PATH_INDEX: tombstone old, write new
    /// - NODE_PATH: write new path for node_id
    /// - ORDERED_CHILDREN: only if parent changes
    ///
    /// No node blob rewrite needed. This is O(1) vs O(N) with embedded paths.
    pub(in crate::repositories::nodes) async fn move_node_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        new_path: &str,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<()> {
        // Get existing node (internal operation - no need to populate has_children)
        let node = self
            .get_impl(tenant_id, repo_id, branch, workspace, id, false)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        // Validation 1: Cannot move root node
        self.validate_not_root_node(&node.path)?;

        // Extract target parent path and new name from new_path
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
        let is_root_node = target_parent_path == "/";
        self.validate_workspace_allows_node_type(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            &node.node_type,
            is_root_node,
        )
        .await?;

        // Validation 4: Check if this child node type is allowed under parent's NodeType schema
        self.validate_parent_allows_child(
            BranchScope::new(tenant_id, repo_id, branch),
            &target_parent_node.node_type,
            &node.node_type,
        )
        .await?;

        // Validation 5: No circular reference (cannot move into self or descendant)
        self.validate_no_circular_reference(&node.path, &target_parent_path)
            .await?;

        // Validation 6: Check for duplicate names in target location
        let is_different_parent = node
            .parent
            .as_ref()
            .map(|p| p != &target_parent_node.id)
            .unwrap_or(true);

        if is_different_parent {
            self.validate_unique_child_name(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &target_parent_node.id,
                &new_name,
            )
            .await?;
        }

        // Save old path for tombstoning
        let old_path = node.path.clone();
        let mut old_parent_id: Option<String> = None;

        // Allocate a new revision for the move operation
        let revision = self.revision_repo.allocate_revision();

        // Prepare WriteBatch for atomic multi-operation move
        let mut batch = WriteBatch::default();

        // Get column family handles
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // === TOMBSTONE OLD PATH INDEX ===
        let old_path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, &old_path, &revision,
        );
        batch.put_cf(cf_path, old_path_key, TOMBSTONE);

        // === TOMBSTONE OLD ORDERED_CHILDREN (if parent is changing) ===
        if node.parent.is_some() {
            let old_parent_path = old_path
                .rsplit_once('/')
                .map(|(parent, _)| {
                    if parent.is_empty() {
                        "/".to_string()
                    } else {
                        parent.to_string()
                    }
                })
                .unwrap_or_else(|| "/".to_string());

            if let Some(old_parent_node) = self
                .get_by_path_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &old_parent_path,
                    None,
                )
                .await?
            {
                old_parent_id = Some(old_parent_node.id.clone());

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

                    // Invalidate cached last-child metadata
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

        // === WRITE NEW PATH INDEX ===
        let new_path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, new_path, &revision,
        );
        batch.put_cf(cf_path, new_path_key, node.id.as_bytes());

        // === WRITE NEW NODE_PATH (node_id -> path reverse lookup) ===
        let node_path_key =
            keys::node_path_key_versioned(tenant_id, repo_id, branch, workspace, id, &revision);
        batch.put_cf(cf_node_path, node_path_key, new_path.as_bytes());

        // === WRITE NEW ORDERED_CHILDREN ===
        let mut new_position: Option<String> = None;
        let new_parent_id_for_index = if target_parent_path != "/" {
            Some(target_parent_node.id.clone())
        } else {
            self.get_by_path_impl(tenant_id, repo_id, branch, workspace, "/", None)
                .await?
                .map(|p| p.id)
        };

        if let Some(ref new_parent_id) = new_parent_id_for_index {
            let existing_label = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                branch,
                workspace,
                new_parent_id,
                &node.id,
            )?;

            let order_label = if let Some(existing) = existing_label {
                existing
            } else {
                let last_label = self.get_last_order_label(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    new_parent_id,
                )?;
                if let Some(ref last) = last_label {
                    match crate::fractional_index::inc(last) {
                        Ok(label) => label,
                        Err(e) => {
                            tracing::warn!(
                                parent_id = %new_parent_id,
                                last_label = %last,
                                error = %e,
                                "Corrupt order label detected in move, falling back to first()"
                            );
                            crate::fractional_index::first()
                        }
                    }
                } else {
                    crate::fractional_index::first()
                }
            };

            new_position = Some(order_label.clone());

            let ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                new_parent_id,
                &order_label,
                &revision,
                &node.id,
            );
            batch.put_cf(cf_ordered, ordered_key, new_name.as_bytes());

            // Update cached last-child metadata
            let metadata_key =
                keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, new_parent_id);
            batch.put_cf(cf_ordered, metadata_key, order_label.as_bytes());
        }

        // Atomic commit - all operations succeed or fail together
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic move failed: {}", e)))?;

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
                    node.id.clone(),
                    old_parent_id,
                    new_parent_id_for_index.clone(),
                    new_position,
                    actor,
                )
                .await?;
        }

        // Index the node change for revision tracking
        self.revision_repo
            .index_node_change(tenant_id, repo_id, &revision, &node.id)
            .await?;

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

        Ok(())
    }
}
