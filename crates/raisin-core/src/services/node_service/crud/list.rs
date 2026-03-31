//! List operations for NodeService
//!
//! Contains list_by_type, list_by_parent, list_root, list_all, and has_children methods.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{
    scope::RepoScope, transactional::TransactionalStorage, BranchRepository, NodeRepository,
    Storage, TreeRepository,
};

use super::super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Lists all nodes of a specific NodeType
    ///
    /// Results are filtered based on user permissions (RLS).
    pub async fn list_by_type(&self, node_type: &str) -> Result<Vec<models::nodes::Node>> {
        // Get committed nodes
        let options = if let Some(rev) = self.revision {
            raisin_storage::ListOptions::at_revision(rev)
        } else {
            raisin_storage::ListOptions::for_api()
        };
        let committed = self
            .storage
            .nodes()
            .list_by_type(self.scope(), node_type, options)
            .await?;

        // Overlay workspace deltas (drafts + tombstones)
        let nodes = self.overlay_workspace_deltas(committed).await?;

        // Apply RLS filtering
        Ok(self.apply_rls_filter_many(nodes))
    }

    /// Lists all nodes with a specific parent ID
    ///
    /// Results are filtered based on user permissions (RLS).
    pub async fn list_by_parent(&self, parent: &str) -> Result<Vec<models::nodes::Node>> {
        // Determine if we should use fast index path or slow tree-based path
        let use_fast_path = if let Some(revision) = self.revision {
            // Check if this revision is the branch HEAD or within branch history
            if let Some(branch_info) = self
                .storage
                .branches()
                .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
                .await?
            {
                if revision <= branch_info.head {
                    // Revision is within this branch's history - use fast index path!
                    tracing::debug!(
                        "list_by_parent: Revision {} <= branch HEAD {}, using fast index path",
                        revision,
                        branch_info.head
                    );
                    true
                } else {
                    // Revision is beyond this branch's HEAD - use slow tree-based path
                    tracing::debug!(
                        "list_by_parent: Revision {} > branch HEAD {}, using tree snapshot path",
                        revision,
                        branch_info.head
                    );
                    false
                }
            } else {
                // Branch doesn't exist - use tree-based path
                false
            }
        } else {
            // No revision specified - use fast index path
            true
        };

        if use_fast_path {
            // FAST PATH: Use branch-scoped indexes
            let options = if let Some(rev) = self.revision {
                raisin_storage::ListOptions::at_revision(rev)
            } else {
                raisin_storage::ListOptions::for_api()
            };
            let committed = self
                .storage
                .nodes()
                .list_by_parent(self.scope(), parent, options)
                .await?;

            // Overlay workspace deltas (drafts + tombstones)
            let nodes = self.overlay_workspace_deltas(committed).await?;

            // Apply RLS filtering
            return Ok(self.apply_rls_filter_many(nodes));
        }

        // SLOW PATH: Use tree-based traversal
        // We need to convert parent ID to path, then use list_children logic
        let parent_node = if parent == models::nodes::ROOT_NODE_ID {
            // Root node - use "/" path
            return self.list_root().await;
        } else {
            // Get parent node to find its path
            self.get(parent).await?
        };

        match parent_node {
            Some(node) => {
                // Use list_children with the parent's path
                self.list_children(&node.path).await
            }
            None => {
                // Parent doesn't exist
                Ok(Vec::new())
            }
        }
    }

    /// Lists root-level nodes with ordering
    ///
    /// Results are filtered based on user permissions (RLS).
    pub async fn list_root(&self) -> Result<Vec<models::nodes::Node>> {
        // MVCC snapshot isolation via revision-aware indexes
        // When self.revision is set (via at_revision()), the repository filters
        // MVCC indexes to only return nodes visible at that revision

        tracing::debug!("SERVICE list_root: max_revision={:?}", self.revision);

        // Get nodes from repository with MVCC filtering
        // Always compute has_children for API responses
        let options = if let Some(rev) = self.revision {
            raisin_storage::ListOptions::for_api_at_revision(rev)
        } else {
            raisin_storage::ListOptions::for_api()
        };
        let committed = self
            .storage
            .nodes()
            .list_root(self.scope(), options)
            .await?;

        // Overlay workspace deltas (drafts)
        let nodes = self.overlay_workspace_deltas(committed).await?;

        // Apply RLS filtering - nodes are already sorted from storage
        Ok(self.apply_rls_filter_many(nodes))
    }

    /// Lists all nodes in the workspace
    /// List all nodes in workspace (DEPRECATED - use list_root() instead)
    ///
    /// Results are filtered based on user permissions (RLS).
    #[deprecated(
        since = "0.1.0",
        note = "Use list_root() or deep_children methods instead"
    )]
    pub async fn list_all(&self) -> Result<Vec<models::nodes::Node>> {
        tracing::debug!("SERVICE list_all: max_revision={:?}", self.revision);

        // Use the ORDERED_CHILDREN index to walk the entire tree in order
        // This preserves the fractional index ordering at every level
        let mut result_nodes = Vec::new();

        // Use a stack for depth-first traversal (iterative, not recursive)
        // Each entry is a parent ID to process
        let mut stack = vec!["/".to_string()];

        while let Some(parent_id) = stack.pop() {
            // Get ordered children from ORDERED_CHILDREN index
            let options = if let Some(rev) = self.revision {
                raisin_storage::ListOptions::at_revision(rev)
            } else {
                raisin_storage::ListOptions::for_api()
            };
            let children = self
                .storage
                .nodes()
                .list_children(self.scope(), &parent_id, options)
                .await?;

            // Push children onto stack in reverse order so they're processed in correct order
            // (stack is LIFO, so reverse order = correct depth-first left-to-right)
            for child in children.iter().rev() {
                stack.push(child.id.clone());
            }

            // Add children to results in original order
            result_nodes.extend(children);
        }

        // Overlay workspace deltas (drafts + tombstones)
        let nodes = self.overlay_workspace_deltas(result_nodes).await?;

        // Apply RLS filtering
        Ok(self.apply_rls_filter_many(nodes))
    }

    /// Check if a node has children
    ///
    /// This method uses different strategies based on the revision context:
    /// - **Fast path (HEAD)**: Uses branch-scoped ordered index for O(1) lookup
    /// - **Slow path (Historical)**: Uses tree snapshot to check if `children_tree_id` is present
    ///
    /// This is more efficient than loading all children just to check if any exist.
    /// Used to populate the `has_children` field in JSON responses.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The ID of the node to check
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the node has children
    /// - `Ok(false)` if the node has no children
    /// - `Err(...)` if there was a storage error
    pub async fn has_children(&self, node_id: &str) -> Result<bool> {
        // Determine if we should use fast index path or slow tree-based path
        let use_fast_path = if let Some(revision) = self.revision {
            // Check if this revision is the branch HEAD or within branch history
            if let Some(branch_info) = self
                .storage
                .branches()
                .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
                .await?
            {
                if revision <= branch_info.head {
                    // Revision is within this branch's history - use fast index path!
                    tracing::debug!(
                        "has_children: Revision {} <= branch HEAD {}, using fast index path",
                        revision,
                        branch_info.head
                    );
                    true
                } else {
                    // Revision is beyond this branch's HEAD - use slow tree-based path
                    tracing::debug!(
                        "has_children: Revision {} > branch HEAD {}, using tree snapshot path",
                        revision,
                        branch_info.head
                    );
                    false
                }
            } else {
                // Branch doesn't exist - use tree-based path
                false
            }
        } else {
            // No revision specified - use fast index path
            true
        };

        if use_fast_path {
            // FAST PATH: Use branch-scoped ordered index (current HEAD or no revision)
            tracing::debug!(
                "has_children: Using fast branch-scoped index for node '{}'",
                node_id
            );
            return self
                .storage
                .nodes()
                .has_children(self.scope(), node_id, self.revision.as_ref())
                .await;
        }

        // SLOW PATH: Use tree-based snapshots (historical revision)
        let revision = self.revision.unwrap(); // Safe because we checked above
        tracing::debug!(
            "has_children: Using tree snapshot for node '{}' at revision {}",
            node_id,
            revision
        );

        // Get the node to find its path
        let node = self.get(node_id).await?;
        let node = match node {
            Some(n) => n,
            None => {
                // Node doesn't exist at this revision
                return Ok(false);
            }
        };

        // Get root tree ID for this revision
        let root_tree_id = self
            .storage
            .trees()
            .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), &revision)
            .await?;

        let root_tree_id = match root_tree_id {
            Some(id) => id,
            None => {
                // Revision doesn't exist
                return Ok(false);
            }
        };

        // Navigate tree to find this node's tree entry
        let tree_entry = self
            .find_tree_entry_for_path(&root_tree_id, &node.path, &revision)
            .await?;

        match tree_entry {
            Some(entry) => {
                // Check if the entry has a children_tree_id (Some = has children, None = no children)
                Ok(entry.children_tree_id.is_some())
            }
            None => {
                // Node not found in tree (shouldn't happen if node.get succeeded)
                Ok(false)
            }
        }
    }
}
