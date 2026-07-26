//! Child listing operations with pagination and revision support.
//!
//! Provides methods for listing children of a node, supporting both
//! fast-path (branch-scoped indexes) and slow-path (tree-based snapshots)
//! queries, as well as paginated variants.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{scope::RepoScope, BranchRepository, NodeRepository, Storage, TreeRepository};
use std::collections::HashMap;

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
                    models::tree::PageCursor::with_kind(
                        entry.entry_key.clone(),
                        Some(revision),
                        models::tree::PageCursorKind::TreeEntry,
                    )
                })
            } else {
                None
            };

            // Apply RLS filtering
            let child_nodes = self.apply_rls_filter_many(child_nodes).await;
            return Ok(models::tree::Page::new(child_nodes, next_cursor));
        }

        // Default: paginate from HEAD via a real keyset seek on the editorial
        // order index.
        //
        // This used to load EVERY child and then `skip_while(name != cursor)` —
        // O(N) per page, and keyed on the node *name*, so a rename or a duplicate
        // name silently broke pagination. The cursor is now the editorial order
        // label, which the index can seek to directly.
        self.list_children_page_from_head(parent_path, cursor, limit)
            .await
    }

    /// Keyset-paginated child listing at HEAD, seeking on the editorial order
    /// index.
    async fn list_children_page_from_head(
        &self,
        parent_path: &str,
        cursor: Option<&models::tree::PageCursor>,
        limit: usize,
    ) -> Result<models::tree::Page<models::nodes::Node>> {
        use models::tree::PageCursorKind;

        if let Some(cursor) = cursor {
            cursor
                .require_kind(PageCursorKind::OrderLabel)
                .map_err(raisin_error::Error::Validation)?;
        }

        let options = if let Some(rev) = self.revision {
            raisin_storage::ListOptions::at_revision(rev)
        } else {
            raisin_storage::ListOptions::for_api()
        };

        // The ordering index keys root-level children under "/", not a node id.
        let parent_id = if parent_path == "/" || parent_path.is_empty() {
            "/".to_string()
        } else {
            match self
                .storage
                .nodes()
                .get_by_path(self.scope(), parent_path, self.revision.as_ref())
                .await?
            {
                Some(parent) => parent.id,
                None => {
                    return Err(raisin_error::Error::NotFound(format!(
                        "Parent node not found at '{parent_path}'"
                    )))
                }
            }
        };

        let mut collected: Vec<models::nodes::Node> = Vec::with_capacity(limit);
        // Label the next page must resume strictly after.
        let mut resume_after: Option<String> = None;
        let mut after_label = cursor.map(|c| c.last_key.clone());
        let mut exhausted = false;

        // RLS is applied per batch, so a batch can yield fewer visible rows than
        // it scanned. Loop to top the page up rather than returning short.
        //
        // Bounded: every round advances `after_label` past everything it scanned,
        // so this terminates. The cap keeps one request from turning into an
        // unbounded scan in a workspace where RLS hides nearly everything — such a
        // page returns short (or empty) with a cursor, and the caller continues.
        const MAX_ROUNDS: usize = 16;
        for _ in 0..MAX_ROUNDS {
            let batch = self
                .storage
                .nodes()
                .list_by_parent_page(
                    self.scope(),
                    &parent_id,
                    after_label.as_deref(),
                    Some(limit),
                    false,
                    options.clone(),
                )
                .await?;

            if batch.is_empty() {
                exhausted = true;
                break;
            }
            let scanned = batch.len();
            let last_scanned_label = batch
                .last()
                .map(|(_, label)| label.clone())
                .expect("batch is non-empty");

            // Keep each node's label so the cursor can point at the last row
            // actually EMITTED. Pointing it at the last row *scanned* would skip
            // any row scanned but not emitted once the page filled — which is
            // exactly how this returned every other child.
            let labels: HashMap<String, String> = batch
                .iter()
                .map(|(node, label)| (node.id.clone(), label.clone()))
                .collect();
            let nodes: Vec<models::nodes::Node> = batch.into_iter().map(|(node, _)| node).collect();
            let visible = self.apply_rls_filter_many(nodes).await;

            for node in visible {
                if collected.len() >= limit {
                    break;
                }
                if let Some(label) = labels.get(&node.id) {
                    resume_after = Some(label.clone());
                }
                collected.push(node);
            }

            if collected.len() >= limit {
                break;
            }

            // Everything visible in this batch was emitted, so it is safe — and
            // necessary — to skip past the hidden rows too, or they would be
            // re-scanned on every round.
            resume_after = Some(last_scanned_label.clone());
            after_label = Some(last_scanned_label);

            if scanned < limit {
                exhausted = true;
                break;
            }
        }

        // A cursor is emitted unless the scan reached the end of the parent.
        // Driven by the SCAN, never by the post-RLS row count: a page shortened
        // by RLS is not the last page.
        let next_cursor = if exhausted {
            None
        } else {
            resume_after.map(|label| {
                models::tree::PageCursor::with_kind(label, None, PageCursorKind::OrderLabel)
            })
        };

        Ok(models::tree::Page::new(collected, next_cursor))
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
            return Ok(self.apply_rls_filter_many(nodes).await);
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
        Ok(self.apply_rls_filter_many(child_nodes).await)
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
            let nodes = self.apply_rls_filter_many(nodes).await;

            let next_cursor = if has_more {
                items.last().map(|entry| {
                    models::tree::PageCursor::with_kind(
                        entry.entry_key.clone(),
                        Some(revision),
                        models::tree::PageCursorKind::TreeEntry,
                    )
                })
            } else {
                None
            };

            return Ok(models::tree::Page::new(nodes, next_cursor));
        }

        // HEAD query — root-level children are keyed under "/" in the editorial
        // order index, so this is the same real keyset seek as any other parent.
        // (Previously: load every root node, then skip_while on the node name.)
        self.list_children_page_from_head("/", cursor, limit).await
    }
}
