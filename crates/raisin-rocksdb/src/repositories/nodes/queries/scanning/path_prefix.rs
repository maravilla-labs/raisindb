//! Path prefix scanning operations.
//!
//! Scans all nodes whose path starts with a given prefix using
//! RocksDB prefix iterators on the PATH_INDEX column family.

use super::super::super::helpers::is_tombstone;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::collections::HashMap;

impl NodeRepositoryImpl {
    /// Scan all nodes whose path starts with the given prefix
    ///
    /// Uses RocksDB prefix iterator on PATH_INDEX column family for O(k) efficiency
    /// where k = number of matching nodes.
    ///
    /// This is significantly faster than list_all() + filter for large workspaces,
    /// especially when the prefix matches a small subset of nodes.
    ///
    /// # Arguments
    /// * `path_prefix` - Path prefix to match (e.g., "/content/")
    /// * `max_revision` - Optional max revision bound for snapshot isolation
    /// * `populate_has_children` - Whether to populate has_children field
    ///
    /// # Performance
    /// - O(k) where k = number of nodes matching prefix
    /// - Uses RocksDB prefix_iterator_cf for efficient scanning
    /// - Only fetches matching nodes, not all nodes in workspace
    pub(in crate::repositories::nodes) async fn scan_by_path_prefix_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path_prefix: &str,
        max_revision: Option<&HLC>,
        populate_has_children: bool,
    ) -> Result<Vec<Node>> {
        tracing::debug!(
            "REPO scan_by_path_prefix_impl: tenant={}, repo={}, branch={}, workspace={}, prefix='{}', max_revision={:?}",
            tenant_id, repo_id, branch, workspace, path_prefix, max_revision
        );

        // Build prefix key for path_index CF
        // This will match ALL paths that start with path_prefix
        //
        // IMPORTANT: We use build() NOT build_prefix() because:
        // - build_prefix() adds a trailing \0: "path\0/house/\0"
        // - build() does not: "path\0/house/"
        // - We want to match paths like "/house/room", "/house/kitchen"
        // - "path\0/house/" IS a prefix of "path\0/house/room\0{revision}"
        // - "path\0/house/\0" is NOT a prefix of "path\0/house/room\0{revision}"
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("path")
            .push(path_prefix)
            .build(); // Use build() not build_prefix()!

        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let prefix_clone = prefix.clone();

        // Use ReadOptions to optimize large scans
        // fill_cache=false prevents polluting block cache for large sequential scans
        let mut read_opts = rocksdb::ReadOptions::default();
        read_opts.set_prefix_same_as_start(true); // Optimize prefix scans
        read_opts.fill_cache(false); // Don't pollute cache for large scans

        let iter = self.db.iterator_cf_opt(
            cf_path,
            read_opts,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        // Collect unique node IDs that match the prefix
        // Use HashMap to track the newest revision for each node_id
        let mut node_revisions: HashMap<String, HLC> = HashMap::new();
        // Track paths that have been tombstoned - we must skip older entries for these paths
        let mut tombstoned_paths: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let mut scanned_count = 0;
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            scanned_count += 1;

            // Debug: log the key we're examining
            tracing::trace!(
                "REPO scan_by_path_prefix_impl: examining key {} (prefix len={}, key len={})",
                String::from_utf8_lossy(&key),
                prefix_clone.len(),
                key.len()
            );

            // Verify key still matches our prefix
            if !key.starts_with(&prefix_clone) {
                tracing::debug!(
                    "REPO scan_by_path_prefix_impl: key doesn't match prefix, stopping iteration after {} entries. Key: {:?}, Prefix: {:?}",
                    scanned_count,
                    String::from_utf8_lossy(&key),
                    String::from_utf8_lossy(&prefix_clone)
                );
                break; // Reached end of matching keys
            }

            // Extract path from key for tombstone tracking
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0path\0{actual_path}\0{~revision}
            let path_from_key = keys::decode_path_from_path_index_key(&key);

            // Decode revision from key - we need this for efficient node fetching
            let revision = match keys::decode_revision_from_path_index_key(&key) {
                Some(rev) => rev,
                None => {
                    tracing::warn!("REPO scan_by_path_prefix_impl: failed to decode revision from key, skipping");
                    continue;
                }
            };

            // Check revision constraint if specified
            if let Some(max_rev) = max_revision {
                if &revision > max_rev {
                    tracing::trace!("REPO scan_by_path_prefix_impl: skipping entry at revision {} (exceeds max_revision {})",
                        revision, max_rev);
                    continue; // Skip future revisions
                }
            }

            // Handle tombstones: if this is a tombstone, track the path as deleted
            // so we skip any older (lower revision) entries for the same path
            if is_tombstone(&value) {
                if let Some(ref path) = path_from_key {
                    tracing::trace!(
                        "REPO scan_by_path_prefix_impl: found tombstone for path '{}' at revision {}",
                        path, revision
                    );
                    tombstoned_paths.insert(path.clone());
                }
                continue;
            }

            // Skip entries for paths that have been tombstoned at a newer revision
            if let Some(ref path) = path_from_key {
                if tombstoned_paths.contains(path) {
                    tracing::trace!(
                        "REPO scan_by_path_prefix_impl: skipping entry for tombstoned path '{}' at revision {}",
                        path, revision
                    );
                    continue;
                }
            }

            // Extract node_id from value
            let node_id = String::from_utf8_lossy(&value).to_string();

            // Track the newest revision we've seen for this node
            // Since iterator returns newest-first, first match is the one we want
            node_revisions.entry(node_id).or_insert(revision);
        }

        tracing::debug!(
            "REPO scan_by_path_prefix_impl: scanned {} index entries, found {} unique nodes",
            scanned_count,
            node_revisions.len()
        );

        // Fetch all matching nodes using RocksDB MultiGet for better performance
        // This reduces round trips compared to fetching nodes one-by-one
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;

        // Build keys for batch fetch
        let node_info_vec: Vec<(String, HLC)> = node_revisions.into_iter().collect();
        let keys: Vec<Vec<u8>> = node_info_vec
            .iter()
            .map(|(node_id, revision)| {
                keys::node_key_versioned(tenant_id, repo_id, branch, workspace, node_id, revision)
            })
            .collect();

        tracing::debug!(
            "REPO scan_by_path_prefix_impl: fetching {} nodes with MultiGet",
            keys.len()
        );

        // Fetch all nodes at once with MultiGet - more efficient than individual gets
        let values = self
            .db
            .multi_get_cf(keys.iter().map(|k| (&cf_nodes, k.as_slice())));

        // Deserialize fetched nodes
        let mut nodes = Vec::new();
        for (i, value_result) in values.into_iter().enumerate() {
            if let Ok(Some(value_bytes)) = value_result {
                // Check for tombstone
                if is_tombstone(&value_bytes) {
                    continue;
                }

                // Get node_id and revision for this index
                let (node_id, revision) = match node_info_vec.get(i) {
                    Some((id, rev)) => (id.as_str(), rev),
                    None => continue, // Shouldn't happen, but safety check
                };

                // Deserialize node and materialize path if needed
                let mut node = self.deserialize_node_with_path(
                    &value_bytes,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id,
                    revision,
                )?;

                // Populate has_children if requested
                if populate_has_children {
                    let has_children = self
                        .has_children_impl(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            node_id,
                            Some(revision),
                        )
                        .await?;
                    node.has_children = Some(has_children);
                }

                // Double-check that path actually starts with prefix
                // (This is a safety check - iterator should already filter correctly)
                if node.path.starts_with(path_prefix) {
                    nodes.push(node);
                } else {
                    tracing::warn!(
                        "REPO scan_by_path_prefix_impl: node {} has path '{}' which doesn't start with prefix '{}'",
                        node.id, node.path, path_prefix
                    );
                }
            }
        }

        tracing::info!(
            "REPO scan_by_path_prefix_impl: returning {} nodes for prefix '{}'",
            nodes.len(),
            path_prefix
        );

        Ok(nodes)
    }
}
