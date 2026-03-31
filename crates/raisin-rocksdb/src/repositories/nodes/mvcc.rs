//! MVCC and time-travel query operations

use super::helpers::is_tombstone;
use super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::collections::{HashMap, HashSet};

impl NodeRepositoryImpl {
    /// Get a node at a specific revision
    ///
    /// Returns the node as it existed at the specified revision number.
    /// If the node didn't exist at that revision, returns None.
    pub async fn get_at_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<Node>> {
        let key =
            keys::node_key_versioned(tenant_id, repo_id, branch, workspace, node_id, revision);
        let cf = cf_handle(&self.db, cf::NODES)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                // Check if this is a tombstone
                if is_tombstone(&bytes) {
                    return Ok(None);
                }

                // Deserialize node and materialize path if needed
                let node = self.deserialize_node_with_path(
                    &bytes, tenant_id, repo_id, branch, workspace, node_id, revision,
                )?;
                Ok(Some(node))
            }
            Ok(None) => {
                // Try to find the most recent revision at or before the requested revision
                self.get_node_at_or_before_revision(
                    tenant_id, repo_id, branch, workspace, node_id, revision,
                )
                .await
            }
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    /// Get the most recent revision of a node at or before a specific revision
    async fn get_node_at_or_before_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: &HLC,
    ) -> Result<Option<Node>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        // Iterate through revisions (newest first due to descending encoding)
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let rev = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(e) => {
                    tracing::warn!(
                        target: "rocksb::nodes::revision_lookup",
                        "Skipping key with invalid revision for node_id={}: {}",
                        node_id,
                        e
                    );
                    continue;
                }
            };

            // Check if this revision is at or before the requested revision
            if &rev <= max_revision {
                // Check if it's a tombstone
                if is_tombstone(&value) {
                    return Ok(None);
                }

                // Deserialize node and materialize path if needed
                let node = self.deserialize_node_with_path(
                    &value, tenant_id, repo_id, branch, workspace, node_id, &rev,
                )?;
                return Ok(Some(node));
            }
        }

        Ok(None)
    }

    /// Resolve node id for a path *as of* a specific revision (skips tombstones)
    ///
    /// Returns the node ID that existed at the given path at or before the specified revision.
    /// This is used for time-travel queries where paths may have changed over time.
    fn get_node_id_by_path_as_of(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        revision: &HLC,
    ) -> Result<Option<String>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("path")
            .push(path)
            .build_prefix();

        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_path, prefix);

        for item in iter {
            let (key, val) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let rev = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(e) => {
                    tracing::warn!(
                        "Skipping path index key with invalid revision for path {}: {}",
                        path,
                        e
                    );
                    continue;
                }
            };

            // Find first entry at or before target revision that's not a tombstone
            if &rev <= revision && !is_tombstone(&val) {
                return Ok(Some(String::from_utf8_lossy(&val).to_string()));
            }
        }
        Ok(None)
    }

    /// List children as they existed at a specific revision (time-travel query)
    ///
    /// Returns the children of a parent node as they were ordered at a specific revision.
    /// This enables historical queries to see the state of the tree at any point in time.
    pub async fn list_children_as_of(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        revision: &HLC,
    ) -> Result<Vec<Node>> {
        // Resolve the parent *id* by path as-of the requested revision
        let parent_id = self
            .get_node_id_by_path_as_of(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_path,
                revision,
            )?
            .ok_or_else(|| {
                raisin_error::Error::NotFound("Parent node not found at revision".to_string())
            })?;

        // Get child IDs as of the specified revision
        let child_ids = self
            .list_children_ids_as_of(tenant_id, repo_id, branch, workspace, &parent_id, revision)
            .await?;

        // Fetch each child *as of* that revision, preserving order
        let mut out = Vec::with_capacity(child_ids.len());
        for cid in child_ids {
            if let Some(node) = self
                .get_at_revision(tenant_id, repo_id, branch, workspace, &cid, revision)
                .await?
            {
                out.push(node);
            }
        }
        Ok(out)
    }

    /// Get the full history of a node across all revisions
    ///
    /// Returns a vector of (revision, node) tuples, ordered from newest to oldest.
    /// Tombstones are included as None values to show when the node was deleted.
    pub async fn get_history(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<(HLC, Option<Node>)>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut history = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(e) => {
                    tracing::warn!(
                        "Skipping node history key with invalid revision (node_id={}): {}",
                        node_id,
                        e
                    );
                    continue;
                }
            };

            // Check if it's a tombstone
            if is_tombstone(&value) {
                history.push((revision, None));
            } else {
                // Deserialize node and materialize path if needed
                let node = self.deserialize_node_with_path(
                    &value, tenant_id, repo_id, branch, workspace, node_id, &revision,
                )?;
                history.push((revision, Some(node)));
            }

            if let Some(max) = limit {
                if history.len() >= max {
                    break;
                }
            }
        }

        Ok(history)
    }

    /// List all nodes in a workspace at a specific revision
    ///
    /// Returns the state of all nodes as they existed at the specified revision.
    /// Deleted nodes (tombstones) are excluded.
    pub async fn list_at_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<Vec<Node>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut nodes_map: HashMap<String, Option<Node>> = HashMap::new();

        // Iterate through all versioned node entries
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Parse node_id and revision from key
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 7 {
                let node_id = String::from_utf8_lossy(parts[5]).to_string();
                let node_rev = match keys::extract_revision_from_key(&key) {
                    Ok(rev) => rev,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping node key with invalid revision (node_id={}): {}",
                            node_id,
                            e
                        );
                        continue;
                    }
                };

                // Only process revisions at or before the requested revision
                if &node_rev <= revision && !nodes_map.contains_key(&node_id) {
                    if is_tombstone(&value) {
                        nodes_map.insert(node_id, None);
                    } else {
                        // Deserialize node and materialize path if needed
                        let node = self.deserialize_node_with_path(
                            &value, tenant_id, repo_id, branch, workspace, &node_id, &node_rev,
                        )?;
                        nodes_map.insert(node_id, Some(node));
                    }
                }
            }
        }

        // Filter out tombstones and return only non-deleted nodes
        Ok(nodes_map.into_values().flatten().collect())
    }

    /// List children as of a specific revision (time-travel)
    ///
    /// Returns children in order as they existed at the specified revision.
    /// This enables historical views with proper ordering.
    pub(super) async fn list_children_ids_as_of(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        revision: &HLC,
    ) -> Result<Vec<String>> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        let mut seen_labels = HashSet::new();
        let mut child_ids = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let order_label = String::from_utf8_lossy(parts[6]).to_string();
                let child_id = String::from_utf8_lossy(parts[8]).to_string();

                let node_rev = match keys::extract_revision_from_key(&key) {
                    Ok(rev) => rev,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping ordered-children key with invalid revision (parent={}, child={}): {}",
                            parent_id,
                            child_id,
                            e
                        );
                        continue;
                    }
                };

                if &node_rev <= revision && !seen_labels.contains(&order_label) {
                    seen_labels.insert(order_label);

                    if !is_tombstone(&value) {
                        child_ids.push(child_id);
                    }
                }
            }
        }

        Ok(child_ids)
    }
}
