//! Descendant scanning and bulk operations
//!
//! This module provides functions for:
//! - Scanning descendants in order
//! - Getting nodes at specific revisions
//! - Bulk descendant retrieval
//! - Path prefix scanning

mod bulk_descendants;
mod path_prefix;

use super::super::helpers::is_tombstone;
use super::super::ordering::{join_tree_order, split_tree_order, OrderedScanStart};
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

/// One pending node in a depth-first subtree walk.
///
/// The stack is LIFO, so a sibling group pushed in reverse label order pops
/// lowest-label-first — i.e. in editorial order.
struct Frame {
    node_id: String,
    depth: usize,
    /// Ancestor order labels joined root-first; see `ordering::tree_order`.
    tree_order: String,
}

impl NodeRepositoryImpl {
    pub(crate) fn scan_descendants_ordered_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        root_node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<(Node, usize)>> {
        Ok(self
            .scan_descendants_ordered_paged_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                root_node_id,
                None,
                None,
                max_revision,
            )?
            .into_iter()
            .map(|(node, depth, _tree_order)| (node, depth))
            .collect())
    }

    /// Depth-first (document-order) walk of a subtree, with each node's
    /// `__tree_order` and an optional resume cursor.
    ///
    /// # Document order
    ///
    /// Emission is **pre-order depth-first**: a node is emitted before its
    /// descendants, and a subtree is emitted contiguously. This is the same
    /// order `ORDER BY __tree_order` produces, and the same order the SQL table
    /// scan and `ORDER BY path` traversal already used.
    ///
    /// (This was breadth-first until editorial ordering became a SQL column.
    /// BFS interleaved cousins, so `DESCENDANT_OF` and a full table scan
    /// disagreed about "tree order". Callers that only need parent-before-child
    /// — subtree copy, move, prune — are unaffected, since pre-order DFS
    /// preserves that too.)
    ///
    /// # `tree_order`
    ///
    /// Each node's ancestor order labels, joined by
    /// [`TREE_ORDER_SEPARATOR`], root-first. The traversal root gets `""`.
    /// Sorting these strings byte-wise reproduces this traversal exactly, which
    /// is what makes `__tree_order` a valid keyset cursor over a whole subtree.
    ///
    /// # Resuming
    ///
    /// `after_tree_order` resumes strictly after the node with that path,
    /// without re-walking what came before: the cursor is split back into
    /// per-level labels and the traversal descends that spine, queuing only the
    /// siblings that still come after it at each level. Cost is O(depth) seeks
    /// rather than O(nodes-already-seen).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scan_descendants_ordered_paged_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        root_node_id: &str,
        after_tree_order: Option<&str>,
        limit: Option<usize>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<(Node, usize, String)>> {
        use std::collections::HashSet;

        tracing::trace!(
            "scan_descendants_ordered_paged: root='{}' workspace='{}' after={:?} limit={:?}",
            root_node_id,
            workspace,
            after_tree_order,
            limit
        );

        if limit == Some(0) {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut stack: Vec<Frame> = Vec::new();

        match after_tree_order {
            // Fresh walk: start at the root itself.
            None => stack.push(Frame {
                node_id: root_node_id.to_string(),
                depth: 0,
                tree_order: String::new(),
            }),
            // Resume: rebuild only the frontier that survives the cursor.
            Some(cursor) => {
                self.seed_resumed_traversal(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    root_node_id,
                    cursor,
                    max_revision,
                    &mut stack,
                )?;
            }
        }

        while let Some(frame) = stack.pop() {
            // Cycle guard: the ordering index is a tree by construction, but a
            // corrupt index must not spin forever.
            if !visited.insert(frame.node_id.clone()) {
                continue;
            }

            let node = match self.get_latest_node_at_or_before_revision(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &frame.node_id,
                max_revision,
            )? {
                Some(node) => node,
                None => {
                    // The index can outlive the node (concurrent delete, or a
                    // revision not visible at this bound). Skip it, but still
                    // descend — its children may be visible.
                    tracing::debug!(
                        "scan_descendants_ordered_paged: node '{}' not found, skipping",
                        frame.node_id
                    );
                    self.push_children_frames(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &frame.node_id,
                        frame.depth,
                        &frame.tree_order,
                        OrderedScanStart::Beginning,
                        max_revision,
                        &mut stack,
                    )?;
                    continue;
                }
            };

            result.push((node, frame.depth, frame.tree_order.clone()));

            if let Some(limit) = limit {
                if result.len() >= limit {
                    break;
                }
            }

            self.push_children_frames(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &frame.node_id,
                frame.depth,
                &frame.tree_order,
                OrderedScanStart::Beginning,
                max_revision,
                &mut stack,
            )?;
        }

        Ok(result)
    }

    /// Push a node's children onto the DFS stack in reverse order, so the
    /// lowest-labelled child is visited first.
    #[allow(clippy::too_many_arguments)]
    fn push_children_frames(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        parent_depth: usize,
        parent_tree_order: &str,
        start: OrderedScanStart<'_>,
        max_revision: Option<&HLC>,
        stack: &mut Vec<Frame>,
    ) -> Result<()> {
        let children = self.list_ordered_children_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            start,
            None,
            false,
            max_revision,
        )?;

        for child in children.into_iter().rev() {
            stack.push(Frame {
                node_id: child.child_id,
                depth: parent_depth + 1,
                tree_order: join_tree_order(parent_tree_order, &child.order_label),
            });
        }
        Ok(())
    }

    /// Rebuild the DFS frontier for a resumed traversal.
    ///
    /// Walks the cursor's spine from the root. At each level it queues the
    /// siblings that sort *after* the spine label (they are still pending), then
    /// descends into the spine child itself. Pushing shallow levels first is what
    /// makes the LIFO stack resume in document order: the deepest pending work
    /// pops before shallower siblings.
    ///
    /// The cursor node itself was already emitted, so only its children are
    /// queued — not the node.
    #[allow(clippy::too_many_arguments)]
    fn seed_resumed_traversal(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        root_node_id: &str,
        cursor: &str,
        max_revision: Option<&HLC>,
        stack: &mut Vec<Frame>,
    ) -> Result<()> {
        let labels: Vec<&str> = split_tree_order(cursor);
        if labels.is_empty() {
            // The cursor is the traversal root: everything below it is pending.
            self.push_children_frames(
                tenant_id,
                repo_id,
                branch,
                workspace,
                root_node_id,
                0,
                "",
                OrderedScanStart::Beginning,
                max_revision,
                stack,
            )?;
            return Ok(());
        }

        let mut current_id = root_node_id.to_string();
        let mut current_path = String::new();

        for (level, label) in labels.iter().enumerate() {
            let depth = level; // parent depth for this level's children

            // Siblings strictly after the spine label are still pending. Queued
            // before descending so they pop after the spine's subtree.
            self.push_children_frames(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &current_id,
                depth,
                &current_path,
                OrderedScanStart::After(label),
                max_revision,
                stack,
            )?;

            // Descend into the spine child. An inclusive start lands on the label
            // itself when it still exists.
            let spine = self.list_ordered_children_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &current_id,
                OrderedScanStart::AtOrAfter(label),
                Some(1),
                false,
                max_revision,
            )?;

            match spine.into_iter().next() {
                Some(child) if child.order_label == *label => {
                    current_path = join_tree_order(&current_path, &child.order_label);
                    current_id = child.child_id;
                }
                // The cursor's spine no longer exists — the node was deleted or
                // reordered away since the cursor was issued. Everything queued
                // so far (the after-siblings at each level walked) is still the
                // correct continuation, so stop descending rather than guessing.
                _ => {
                    tracing::debug!(
                        "scan_descendants_ordered_paged: cursor spine label {:?} no longer present \
                         under '{}'; resuming from queued siblings",
                        label,
                        current_id
                    );
                    return Ok(());
                }
            }
        }

        // Reached the cursor node itself. It was already emitted; queue only its
        // children, which pop before any of the sibling groups above.
        self.push_children_frames(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &current_id,
            labels.len(),
            &current_path,
            OrderedScanStart::Beginning,
            max_revision,
            stack,
        )?;

        Ok(())
    }

    /// Helper to get node at or before specific revision (for MVCC time-travel)
    pub(in crate::repositories::nodes) fn get_latest_node_at_or_before_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        // Build prefix for all versions of this node
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .push(node_id)
            .build_prefix();

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let iter = self.db.prefix_iterator_cf(cf_nodes, prefix);

        // Iterate through versions (newest first due to descending revision encoding)
        // Use iter.next() pattern which is proven to work reliably
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Parse revision from key first - we need it for both tombstone and bound checks
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(_) => continue, // Skip keys with invalid revisions
            };

            // Check if this revision is within our bound
            if let Some(max_rev) = max_revision {
                if &revision > max_rev {
                    continue; // Skip versions after max_revision
                }
            }

            // Handle tombstones - if tombstone is within our revision bound, node is deleted
            // Only for time-travel queries (max_revision specified) should we potentially
            // skip tombstones that are AFTER the max_revision (already handled above)
            if is_tombstone(&value) {
                // Tombstone is within our time window - node is deleted at this point
                return Ok(None);
            }

            // Found valid node at acceptable revision - deserialize and materialize path if needed
            let node = self.deserialize_node_with_path(
                &value, tenant_id, repo_id, branch, workspace, node_id, &revision,
            )?;

            return Ok(Some(node));
        }

        // No valid node found
        Ok(None)
    }
}
