//! Bulk descendant fetching operations.
//!
//! Efficient batch retrieval of all descendants under a given path
//! using RocksDB prefix scans and MultiGet for optimized I/O.

use super::super::super::helpers::is_tombstone;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::collections::HashMap;

impl NodeRepositoryImpl {
    /// Bulk fetch all descendants using efficient RocksDB prefix scans.
    ///
    /// This method is optimized for building deep trees without recursive individual fetches.
    /// It uses a single RocksDB prefix scan on the PATH_INDEX CF to fetch all descendants.
    ///
    /// # Performance
    ///
    /// - O(k) where k = number of descendants
    /// - Single RocksDB prefix scan instead of recursive individual gets
    /// - 10-100x faster than recursive fetching for deep trees
    ///
    /// # Arguments
    ///
    /// * `parent_path` - Root path (e.g., "/content" or "/" for all children)
    /// * `max_depth` - Maximum depth relative to parent_path (0 = direct children only)
    /// * `max_revision` - Optional max revision for snapshot isolation
    ///
    /// # Returns
    ///
    /// HashMap where key is the full node path and value is the Node.
    pub(in crate::repositories::nodes) async fn get_descendants_bulk_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<HashMap<String, Node>> {
        tracing::debug!(
            "REPO get_descendants_bulk_impl: tenant={}, repo={}, branch={}, ws={}, parent_path='{}', max_depth={}, max_revision={:?}",
            tenant_id, repo_id, branch, workspace, parent_path, max_depth, max_revision
        );

        // Normalize parent path
        let search_prefix = if parent_path == "/" || parent_path.is_empty() {
            "/".to_string()
        } else {
            // Ensure it ends with / to match children
            if parent_path.ends_with('/') {
                parent_path.to_string()
            } else {
                format!("{}/", parent_path)
            }
        };

        // Calculate the base depth (number of slashes in parent_path)
        let base_depth = if parent_path == "/" {
            0
        } else {
            parent_path.matches('/').count()
        };

        // Build prefix key for path_index CF
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("path")
            .push(&search_prefix)
            .build(); // Use build() not build_prefix()!

        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let prefix_clone = prefix.clone();

        // Use ReadOptions to optimize large scans (descendants can be large)
        let mut read_opts = rocksdb::ReadOptions::default();
        read_opts.set_prefix_same_as_start(true);
        read_opts.fill_cache(false); // Don't pollute cache for bulk operations

        let iter = self.db.iterator_cf_opt(
            cf_path,
            read_opts,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        // Collect unique node IDs with their paths and revisions
        // Use HashMap to track the newest revision for each node_id
        let mut node_info: HashMap<String, (String, HLC)> = HashMap::new(); // node_id -> (path, revision)

        // Track paths that have been tombstoned - we must skip older entries for these paths
        // since iterator returns newest-first, a tombstone means the node was deleted
        let mut tombstoned_paths: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let mut scanned_count = 0;
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            scanned_count += 1;

            // Verify key still matches our prefix
            if !key.starts_with(&prefix_clone) {
                tracing::debug!(
                    "REPO get_descendants_bulk_impl: prefix mismatch, stopping after {} entries",
                    scanned_count
                );
                break;
            }

            // Decode the path from the key first (needed for tombstone tracking)
            // Key structure: {tenant}\0{repo}\0{branch}\0{ws}\0path\0{path}\0{~revision}
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() < 7 {
                tracing::warn!("REPO get_descendants_bulk_impl: malformed key, skipping");
                continue;
            }
            let node_path = String::from_utf8_lossy(parts[5]).to_string();

            // Handle tombstones: if this is a tombstone, track the path as deleted
            // Since iterator returns newest-first, tombstone means this path is deleted
            if is_tombstone(&value) {
                tombstoned_paths.insert(node_path);
                continue;
            }

            // Skip entries for paths that have been tombstoned at a newer revision
            if tombstoned_paths.contains(&node_path) {
                continue;
            }

            // Extract node_id from value
            let node_id = String::from_utf8_lossy(&value).to_string();

            // Check depth constraint
            let node_depth = node_path.matches('/').count();
            let relative_depth = node_depth - base_depth;
            if max_depth < u32::MAX && relative_depth > max_depth as usize {
                tracing::trace!(
                    "REPO get_descendants_bulk_impl: skipping node_id={} path='{}' (depth {} > max {})",
                    node_id, node_path, relative_depth, max_depth
                );
                continue;
            }

            // Decode revision from key
            let revision = match keys::decode_revision_from_path_index_key(&key) {
                Some(rev) => rev,
                None => {
                    tracing::warn!(
                        "REPO get_descendants_bulk_impl: failed to decode revision, skipping"
                    );
                    continue;
                }
            };

            // Check revision constraint
            if let Some(max_rev) = max_revision {
                if &revision > max_rev {
                    tracing::trace!(
                        "REPO get_descendants_bulk_impl: skipping node_id={} at revision {} (exceeds max {})",
                        node_id, revision, max_rev
                    );
                    continue;
                }
            }

            // Track the newest revision for this node
            // Since iterator returns newest-first, first match is the one we want
            node_info.entry(node_id).or_insert((node_path, revision));
        }

        tracing::debug!(
            "REPO get_descendants_bulk_impl: scanned {} index entries, found {} unique nodes within depth {}",
            scanned_count, node_info.len(), max_depth
        );

        // Use RocksDB MultiGet for efficient batch fetching
        // Convert to Vec to maintain alignment between keys and paths
        let node_info_vec: Vec<(String, String, HLC)> = node_info
            .into_iter()
            .map(|(node_id, (node_path, revision))| (node_id, node_path, revision))
            .collect();

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let keys: Vec<Vec<u8>> = node_info_vec
            .iter()
            .map(|(node_id, _, revision)| {
                keys::node_key_versioned(tenant_id, repo_id, branch, workspace, node_id, revision)
            })
            .collect();

        tracing::debug!(
            "REPO get_descendants_bulk_impl: fetching {} nodes with MultiGet",
            keys.len()
        );

        // Fetch all nodes at once with MultiGet - only reading values at the last moment
        let values = self
            .db
            .multi_get_cf(keys.iter().map(|k| (&cf_nodes, k.as_slice())));

        // Build result map from fetched nodes
        let mut result = HashMap::new();
        for (i, value_result) in values.into_iter().enumerate() {
            if let Ok(Some(value_bytes)) = value_result {
                // Get node_id, path, and revision for this index
                let (node_id, node_path, revision) = match node_info_vec.get(i) {
                    Some((id, path, rev)) => (id.as_str(), path, rev),
                    None => continue, // Shouldn't happen, but safety check
                };

                // Deserialize node and materialize path if needed
                let node = self.deserialize_node_with_path(
                    &value_bytes,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id,
                    revision,
                )?;

                // Verify the path matches (safety check)
                if node.path.starts_with(&search_prefix) {
                    result.insert(node_path.clone(), node);
                } else {
                    tracing::warn!(
                        "REPO get_descendants_bulk_impl: node {} has path '{}' which doesn't start with '{}'",
                        node.id, node.path, search_prefix
                    );
                }
            }
        }

        tracing::info!(
            "REPO get_descendants_bulk_impl: returning {} nodes for parent_path='{}' depth={} (used MultiGet for batch fetch)",
            result.len(), parent_path, max_depth
        );

        Ok(result)
    }
}
