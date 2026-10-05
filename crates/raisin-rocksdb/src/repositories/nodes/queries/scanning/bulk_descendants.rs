//! Bulk descendant fetching operations.
//!
//! Efficient batch retrieval of all descendants under a given path
//! using one PATH_INDEX prefix scan and one bounded NODES seek per node.

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
        let (search_prefix, node_info) = self.descendant_index_entries(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
            max_revision,
        )?;

        // Each node's NEWEST version at or below the bound — not the version
        // at its PATH_INDEX entry's revision, which is only where the node last
        // got its path: a node edited after that (no move) used to come back
        // with its old properties. Same rule, and same seek, as a point read.
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let mut result = HashMap::new();
        for (node_id, (node_path, path_revision)) in node_info {
            let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, &node_id);
            let Some((blob_revision, bytes)) =
                crate::mvcc_read::newest_at_or_before(&self.db, cf_nodes, &prefix, max_revision)?
            else {
                continue;
            };
            if is_tombstone(&bytes) {
                continue;
            }

            // The path as of the read: the bound, or else whichever is newer
            // of the path entry and the blob.
            let path_at = match max_revision {
                Some(bound) => *bound,
                None => blob_revision.max(path_revision),
            };
            let node = self.deserialize_node_with_path(
                &bytes,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node_id,
                &path_at,
                &blob_revision,
            )?;

            // Verify the path matches (safety check)
            if node.path.starts_with(&search_prefix) {
                result.insert(node_path, node);
            } else {
                tracing::warn!(
                    "REPO get_descendants_bulk_impl: node {} has path '{}' which doesn't start with '{}'",
                    node.id, node.path, search_prefix
                );
            }
        }

        tracing::debug!(
            "REPO get_descendants_bulk_impl: returning {} nodes for parent_path='{}' depth={} ",
            result.len(),
            parent_path,
            max_depth
        );

        Ok(result)
    }

    /// The PATH_INDEX half of [`Self::get_descendants_bulk_impl`]: every
    /// descendant within `max_depth` as `node_id -> (path, revision)`, with no
    /// node blob read. Returns the normalized search prefix alongside.
    ///
    /// Callers that only need the SHAPE of a subtree (which node has children)
    /// use this directly; the deep readers use it to skip ORDERED_CHILDREN
    /// scans for nodes known to be leaves.
    #[allow(clippy::type_complexity)]
    pub(in crate::repositories::nodes) fn descendant_index_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<(String, HashMap<String, (String, HLC)>)> {
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

        // Paths whose state at `max_revision` is already decided. Entries of
        // one path run newest first, so the first one AT OR BELOW the bound
        // decides it: a tombstone means the path was gone by then, a node id
        // means it held that node. A tombstone NEWER than the bound decides
        // nothing — it used to be recorded first and so hid, from every
        // time-travel read, a node that was moved or deleted only later.
        let mut decided_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

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

            // Key structure: {tenant}\0{repo}\0{branch}\0{ws}\0path\0{path}\0{~revision}
            // (the path itself never contains a NUL, so part 5 is the path)
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() < 7 {
                tracing::warn!("REPO get_descendants_bulk_impl: malformed key, skipping");
                continue;
            }
            let node_path = String::from_utf8_lossy(parts[5]).to_string();

            // Check depth constraint
            let node_depth = node_path.matches('/').count();
            let relative_depth = node_depth - base_depth;
            if max_depth < u32::MAX && relative_depth > max_depth as usize {
                continue;
            }

            // Decode revision from key; skip entries above the bound
            let revision = match keys::decode_revision_from_path_index_key(&key) {
                Some(rev) => rev,
                None => {
                    tracing::warn!(
                        "REPO get_descendants_bulk_impl: failed to decode revision, skipping"
                    );
                    continue;
                }
            };
            if max_revision.is_some_and(|max_rev| &revision > max_rev) {
                continue;
            }

            if !decided_paths.insert(node_path.clone()) || is_tombstone(&value) {
                continue;
            }

            // Extract node_id from value; the first path seen for a node wins
            let node_id = String::from_utf8_lossy(&value).to_string();
            node_info.entry(node_id).or_insert((node_path, revision));
        }

        tracing::debug!(
            "REPO get_descendants_bulk_impl: scanned {} index entries, found {} unique nodes within depth {}",
            scanned_count, node_info.len(), max_depth
        );

        Ok((search_prefix, node_info))
    }
}
