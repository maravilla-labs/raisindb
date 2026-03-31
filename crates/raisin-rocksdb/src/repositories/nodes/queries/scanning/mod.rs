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
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

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
        use std::collections::{HashSet, VecDeque};

        tracing::trace!(
            "scan_descendants_ordered called with root_node_id='{}' in workspace='{}'",
            root_node_id,
            workspace
        );

        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        // Get root node
        let root_node = match self.get_latest_node_at_or_before_revision(
            tenant_id,
            repo_id,
            branch,
            workspace,
            root_node_id,
            max_revision,
        )? {
            Some(node) => node,
            None => {
                // Root node doesn't exist - return empty descendants list
                // This can happen legitimately when:
                // 1. Node has no descendants (leaf node)
                // 2. Node was just created and index hasn't propagated
                // 3. Concurrent deletion occurred
                tracing::warn!(
                    "scan_descendants_ordered: Root node '{}' not found, returning empty list",
                    root_node_id
                );
                return Ok(Vec::new());
            }
        };

        // Start BFS from root
        queue.push_back((root_node, 0_usize));

        while let Some((node, depth)) = queue.pop_front() {
            let node_id = node.id.clone();

            // Skip if already visited (prevents cycles)
            if !visited.insert(node_id.clone()) {
                continue;
            }

            result.push((node.clone(), depth));

            // Scan children using ORDERED_CHILDREN prefix iterator
            let prefix =
                keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, &node_id);
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            let prefix_clone = prefix.clone();
            let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

            let mut seen_order_labels = HashSet::new();

            for item in iter {
                let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                // Verify key starts with our prefix
                if !key.starts_with(&prefix_clone) {
                    break;
                }

                // Parse key: {prefix}\0{order_label}\0{~revision-16bytes}\0{child_id}
                // Note: HLC is 16 bytes and may contain null bytes, so we can't simply split by nulls
                let suffix = &key[prefix_clone.len()..];

                // Find the end of order_label (first null byte)
                let order_label_end = match suffix.iter().position(|&b| b == 0) {
                    Some(pos) => pos,
                    None => continue, // Invalid key format
                };

                // Extract order_label
                let order_label = std::str::from_utf8(&suffix[..order_label_end]).map_err(|e| {
                    raisin_error::Error::storage(format!("Invalid order_label: {}", e))
                })?;

                // Skip order_label + null byte, then skip 16 bytes for HLC, then skip one null byte
                let child_id_start = order_label_end + 1 + 16 + 1;
                if suffix.len() < child_id_start {
                    continue; // Invalid key format
                }

                // Extract child_id (rest of the key)
                let child_id = std::str::from_utf8(&suffix[child_id_start..]).map_err(|e| {
                    raisin_error::Error::storage(format!("Invalid child_id: {}", e))
                })?;

                // Due to descending revision encoding, we see newest version first
                // Skip if we've already seen this order_label (means we already have the HEAD version)
                // IMPORTANT: We must mark as seen BEFORE checking tombstone, otherwise
                // an older non-tombstoned entry would be processed after we skip the tombstone
                if !seen_order_labels.insert(order_label.to_string()) {
                    continue;
                }

                // Skip if value is tombstone (but we've already marked order_label as seen above,
                // so older entries for this position won't resurrect deleted children)
                if is_tombstone(&value) {
                    continue;
                }

                // Load child node
                if let Some(child_node) = self.get_latest_node_at_or_before_revision(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    child_id,
                    max_revision,
                )? {
                    queue.push_back((child_node, depth + 1));
                }
            }
        }

        Ok(result)
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
