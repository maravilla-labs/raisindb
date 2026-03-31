//! Child listing operations with pagination and revision support.
//!
//! Provides methods for listing children of a node, supporting both
//! fast-path (branch-scoped indexes) and slow-path (tree-based snapshots)
//! queries, as well as paginated variants.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{scope::RepoScope, BranchRepository, NodeRepository, Storage, TreeRepository};

use super::super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Lists all children of a node at a given depth with pagination support
    ///
    /// # Arguments
    /// * `parent_path` - Path of the parent node
    /// * `cursor` - Optional cursor for pagination
    /// * `limit` - Maximum number of items to return (clamped to 1-1000)
    ///
    /// # Returns
    /// Page<Node> with items and optional next_cursor
    pub async fn list_children_page(
        &self,
        parent_path: &str,
        cursor: Option<&models::tree::PageCursor>,
        limit: usize,
    ) -> Result<models::tree::Page<models::nodes::Node>> {
        // Clamp limit to reasonable bounds
        let limit = limit.clamp(1, 1000);

        // Determine which revision to query
        let target_revision = cursor.and_then(|c| c.revision).or(self.revision);

        // If viewing a specific revision, use tree-based query
        if let Some(revision) = target_revision {
            // Special case: root level
            if parent_path == "/" || parent_path.is_empty() {
                return self.list_root_page(cursor, limit).await;
            }

            // Get root tree ID for this revision
            let root_tree_id = self
                .storage
                .trees()
                .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), &revision)
                .await?;

            let root_tree_id = match root_tree_id {
                Some(id) => id,
                None => {
                    // Revision doesn't exist - return empty page
                    return Ok(models::tree::Page::new(vec![], None));
                }
            };

            // Navigate tree to find parent's children_tree_id
            let parent_tree_id = self
                .find_children_tree_id_for_path(&root_tree_id, parent_path, &revision)
                .await?;

            let parent_tree_id = match parent_tree_id {
                Some(id) => id,
                None => {
                    // Parent has no children - return empty page
                    return Ok(models::tree::Page::new(vec![], None));
                }
            };

            // Get entries with pagination
            let start_after = cursor.map(|c| c.last_key.as_str());
            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &parent_tree_id,
                    start_after,
                    limit + 1,
                )
                .await?;

            let has_more = entries.len() > limit;
            let items: Vec<_> = entries.into_iter().take(limit).collect();

            // Convert entries to nodes by fetching from NODES CF
            let mut child_nodes = Vec::new();
            for entry in &items {
                if let Some(node) = self
                    .storage
                    .nodes()
                    .get(self.scope(), &entry.node_id, Some(&revision))
                    .await?
                {
                    child_nodes.push(node);
                }
            }

            // Create next cursor if there are more results
            let next_cursor = if has_more {
                items.last().map(|entry| {
                    models::tree::PageCursor::new(entry.entry_key.clone(), Some(revision))
                })
            } else {
                None
            };

            // Apply RLS filtering
            let child_nodes = self.apply_rls_filter_many(child_nodes);
            return Ok(models::tree::Page::new(child_nodes, next_cursor));
        }

        // Default: paginate from HEAD
        // For HEAD queries, we need to use NodeRepository methods
        // Since NodeRepository doesn't have native pagination, we fetch limit+1 and check
        let options = if let Some(rev) = self.revision {
            raisin_storage::ListOptions::at_revision(rev)
        } else {
            raisin_storage::ListOptions::for_api()
        };
        let all_children = self
            .storage
            .nodes()
            .list_children(self.scope(), parent_path, options)
            .await?;

        // Apply RLS filtering before pagination
        let all_children = self.apply_rls_filter_many(all_children);

        // Apply cursor filtering
        let start_after = cursor.map(|c| c.last_key.as_str());
        let filtered: Vec<_> = if let Some(after_key) = start_after {
            all_children
                .into_iter()
                .skip_while(|n| n.name != after_key)
                .skip(1) // Skip the cursor item itself
                .collect()
        } else {
            all_children
        };

        let has_more = filtered.len() > limit;
        let items: Vec<_> = filtered.into_iter().take(limit).collect();

        let next_cursor = if has_more {
            items
                .last()
                .map(|node| models::tree::PageCursor::new(node.name.clone(), None))
        } else {
            None
        };

        Ok(models::tree::Page::new(items, next_cursor))
    }

    /// Lists all children of a node at a given depth
    ///
    /// Results are filtered based on user permissions (RLS).
    pub async fn list_children(&self, parent_path: &str) -> Result<Vec<models::nodes::Node>> {
        // Determine if we should use fast index path or slow tree-based path
        let use_fast_path = if let Some(revision) = &self.revision {
            // Check if this revision is the branch HEAD or within branch history
            if let Some(branch_info) = self
                .storage
                .branches()
                .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
                .await?
            {
                if revision <= &branch_info.head {
                    tracing::debug!(
                        "list_children: Revision {:?} <= branch HEAD {:?}, using fast index path",
                        revision,
                        branch_info.head
                    );
                    true
                } else {
                    tracing::debug!(
                        "list_children: Revision {:?} > branch HEAD {:?}, using tree snapshot path",
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
            // FAST PATH: Use branch-scoped indexes (current HEAD or no revision)
            tracing::debug!(
                "list_children: Using fast branch-scoped index for path '{}'",
                parent_path
            );
            // Always compute has_children for API responses
            let options = if let Some(rev) = self.revision {
                raisin_storage::ListOptions::for_api_at_revision(rev)
            } else {
                raisin_storage::ListOptions::for_api()
            };
            let nodes = self
                .storage
                .nodes()
                .list_children(self.scope(), parent_path, options)
                .await?;

            // Apply RLS filtering
            return Ok(self.apply_rls_filter_many(nodes));
        }

        // SLOW PATH: Use tree-based snapshots (historical revision)
        let revision = self.revision.as_ref().unwrap(); // Safe because we checked above

        // Special case: root level
        if parent_path == "/" || parent_path.is_empty() {
            return self.list_root().await;
        }

        tracing::debug!(
            "list_children: Using tree snapshot for path '{}' at revision {:?}",
            parent_path,
            revision
        );

        // Get root tree ID for this revision
        let root_tree_id = self
            .storage
            .trees()
            .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), revision)
            .await?;

        let root_tree_id = match root_tree_id {
            Some(id) => id,
            None => {
                // Revision doesn't exist
                return Ok(Vec::new());
            }
        };

        // Navigate tree to find parent's tree entry
        let parent_tree_id = self
            .find_children_tree_id_for_path(&root_tree_id, parent_path, revision)
            .await?;

        let parent_tree_id = match parent_tree_id {
            Some(id) => id,
            None => {
                // Parent has no children tree (empty directory)
                return Ok(Vec::new());
            }
        };

        // Get all entries from parent's children tree
        let entries = self
            .storage
            .trees()
            .iter_tree(
                RepoScope::new(&self.tenant_id, &self.repo_id),
                &parent_tree_id,
                None,
                10000,
            )
            .await?;

        let mut child_nodes = Vec::new();

        // For each entry, get the node from NODES CF at this revision
        for entry in entries {
            if let Some(node) = self
                .storage
                .nodes()
                .get(self.scope(), &entry.node_id, Some(revision))
                .await?
            {
                child_nodes.push(node);
            }
        }

        // Apply RLS filtering
        Ok(self.apply_rls_filter_many(child_nodes))
    }

    /// Lists root-level nodes with pagination
    async fn list_root_page(
        &self,
        cursor: Option<&models::tree::PageCursor>,
        limit: usize,
    ) -> Result<models::tree::Page<models::nodes::Node>> {
        // Determine which revision to query
        let target_revision = cursor.and_then(|c| c.revision).or(self.revision);

        if let Some(revision) = target_revision {
            // Get root tree ID for this revision
            let root_tree_id = self
                .storage
                .trees()
                .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), &revision)
                .await?;

            let root_tree_id = match root_tree_id {
                Some(id) => id,
                None => {
                    return Ok(models::tree::Page::new(vec![], None));
                }
            };

            // Get entries with pagination
            let start_after = cursor.map(|c| c.last_key.as_str());
            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &root_tree_id,
                    start_after,
                    limit + 1,
                )
                .await?;

            let has_more = entries.len() > limit;
            let items: Vec<_> = entries.into_iter().take(limit).collect();

            // Convert entries to nodes by fetching from NODES CF
            let mut nodes = Vec::new();
            for entry in &items {
                if let Some(node) = self
                    .storage
                    .nodes()
                    .get(self.scope(), &entry.node_id, Some(&revision))
                    .await?
                {
                    nodes.push(node);
                }
            }

            // Apply RLS filtering
            let nodes = self.apply_rls_filter_many(nodes);

            let next_cursor = if has_more {
                items.last().map(|entry| {
                    models::tree::PageCursor::new(entry.entry_key.clone(), Some(revision))
                })
            } else {
                None
            };

            return Ok(models::tree::Page::new(nodes, next_cursor));
        }

        // HEAD query - use list_root and manually paginate
        let all_nodes = self.list_root().await?;

        let start_after = cursor.map(|c| c.last_key.as_str());
        let filtered: Vec<_> = if let Some(after_key) = start_after {
            all_nodes
                .into_iter()
                .skip_while(|n| n.name != after_key)
                .skip(1)
                .collect()
        } else {
            all_nodes
        };

        let has_more = filtered.len() > limit;
        let items: Vec<_> = filtered.into_iter().take(limit).collect();

        let next_cursor = if has_more {
            items
                .last()
                .map(|node| models::tree::PageCursor::new(node.name.clone(), None))
        } else {
            None
        };

        Ok(models::tree::Page::new(items, next_cursor))
    }
}
