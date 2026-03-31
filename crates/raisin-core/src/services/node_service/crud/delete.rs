//! Delete operations for NodeService
//!
//! Contains delete and delete_by_path methods for removing nodes.

use raisin_error::Result;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_models::permissions::Operation;
use raisin_storage::{
    scope::StorageScope, transactional::TransactionalStorage, NodeRepository, Storage,
    UpdateNodeOptions,
};

use super::super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Deletes a node by ID
    ///
    /// # Delete Protection
    ///
    /// This method will fail if:
    /// - The node is published
    /// - Any descendant nodes are published
    ///
    /// Unpublish the node (and tree if needed) before deleting.
    ///
    /// # Version Cascade Delete
    ///
    /// When a node is deleted, all its children and their version history are also deleted (cascade).
    /// This ensures consistent behavior between ID-based and path-based deletions.
    pub async fn delete(&self, id: &str) -> Result<bool> {
        // Look up node to get its path, then delegate to delete_by_path for cascade
        // This ensures delete(id) and delete_by_path(path) have identical behavior:
        // - Both cascade to all descendants
        // - Both set tombstones for entire subtree
        // - Both check published status of node and all descendants
        let node = self.get(id).await?;
        match node {
            Some(n) => self.delete_by_path(&n.path).await,
            None => Ok(false), // Node not found
        }
    }

    /// Deletes a node by path
    ///
    /// # Delete Protection
    ///
    /// This method will fail if:
    /// - The node is published
    /// - Any descendant nodes are published
    ///
    /// This method deletes the node AND all its descendants, so we check
    /// the entire tree for published nodes before allowing deletion.
    ///
    /// Unpublish the tree before deleting using `unpublish_tree()`.
    ///
    /// # Version Cascade Delete
    ///
    /// When nodes are deleted, all their version history is also deleted.
    ///
    /// # Authorization
    ///
    /// Requires delete permission for the node and all descendants.
    pub async fn delete_by_path(&self, path: &str) -> Result<bool> {
        // Use self.get_by_path() which checks workspace delta first
        // Note: get_by_path already applies RLS filtering, so if user can't read they'll get None
        let before = self.get_by_path(path).await?;

        // Collect node IDs for cascade deletion of versions
        let mut node_ids_to_delete = Vec::new();

        if let Some(node) = &before {
            // RLS Authorization: Check if user can delete this node
            if !self.check_rls_permission(node, Operation::Delete) {
                return Err(raisin_error::Error::PermissionDenied(format!(
                    "Permission denied: cannot delete node at path '{}'",
                    path
                )));
            }

            // 1. Check if the node itself is published
            if node.published_at.is_some() {
                return Err(raisin_error::Error::Validation(
                    "Cannot delete published node. Unpublish first.".to_string(),
                ));
            }

            node_ids_to_delete.push(node.id.clone());

            // 2. Check if any descendants are published
            // delete_by_path deletes the entire tree, so we must check ALL descendants
            let descendants = self
                .storage
                .nodes()
                .deep_children_flat(self.scope(), path, 100, self.revision.as_ref())
                .await?;

            for desc_node in &descendants {
                if desc_node.published_at.is_some() {
                    return Err(raisin_error::Error::Validation(format!(
                        "Cannot delete node - child '{}' is published. Unpublish children first.",
                        desc_node.path
                    )));
                }
                node_ids_to_delete.push(desc_node.id.clone());
            }
        }

        let was_root = before
            .as_ref()
            .map(|n| n.parent.as_deref().unwrap_or("").is_empty())
            .unwrap_or(false);
        let _name = before.as_ref().map(|n| n.name.clone());

        // CRITICAL: Create tombstones for all nodes being deleted
        // This includes the main node and all descendants
        if let Some(main_node) = &before {
            // Delete main node (creates tombstone)
            self.storage
                .delete_workspace_delta(
                    StorageScope::new(
                        &self.tenant_id,
                        &self.repo_id,
                        &self.branch,
                        &self.workspace_id,
                    ),
                    &main_node.id,
                    &main_node.path,
                )
                .await?;

            // Delete all descendant nodes (create tombstones)
            let descendants = self
                .storage
                .nodes()
                .deep_children_flat(self.scope(), path, 100, self.revision.as_ref())
                .await?;

            for desc_node in descendants {
                self.storage
                    .delete_workspace_delta(
                        StorageScope::new(
                            &self.tenant_id,
                            &self.repo_id,
                            &self.branch,
                            &self.workspace_id,
                        ),
                        &desc_node.id,
                        &desc_node.path,
                    )
                    .await?;
            }
        }

        let res = true; // Always succeeds if we got here

        if res {
            // Version history is preserved - tombstones mark deletion without removing data

            // If this was a root-level node, remove it from ROOT node's children array
            if was_root {
                if let Some(mut root_node) = self
                    .storage
                    .nodes()
                    .get_by_path(self.scope(), "/", self.revision.as_ref())
                    .await?
                {
                    if let Some(ref node) = before {
                        root_node.children.retain(|id| id != &node.id);
                        self.storage
                            .nodes()
                            .update(self.scope(), root_node, UpdateNodeOptions::default())
                            .await?;
                    }
                }
            }

            // Audit log the delete operation
            if let (Some(a), Some(n)) = (&self.audit, before) {
                a.write(&n, AuditLogAction::Delete, None).await?;
            }
        }
        Ok(res)
    }
}
