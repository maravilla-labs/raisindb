//! Node lookup operations by path
//!
//! This module provides functions for looking up and deleting nodes by their path.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Get node by path using PATH_INDEX
    pub(crate) async fn get_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        tracing::info!(
            "REPO get_by_path_impl: tenant={}, repo={}, branch={}, workspace={}, path={}",
            tenant_id,
            repo_id,
            branch,
            workspace,
            path
        );

        // Build prefix for this path across all revisions
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
        let mut iter = self.db.prefix_iterator_cf(cf_path, prefix);

        // MVCC semantics with time-travel support:
        // Keys are sorted by revision descending (newest first)
        // For HEAD queries (max_revision = None): first entry is current state
        // For time-travel queries: skip entries newer than max_revision
        // If the relevant entry is a tombstone → path is deleted → return None
        // If the relevant entry is node_id → path exists → return that node
        let node_id = loop {
            match iter.next() {
                Some(Ok((key, bytes))) => {
                    // Verify key actually starts with our prefix
                    if !key.starts_with(&prefix_clone) {
                        tracing::info!(
                            "REPO get_by_path_impl: key doesn't match prefix, path not found"
                        );
                        return Ok(None);
                    }

                    // For time-travel queries, skip entries newer than max_revision
                    if let Some(max_rev) = max_revision {
                        let revision = match keys::extract_revision_from_key(&key) {
                            Ok(rev) => rev,
                            Err(e) => {
                                tracing::warn!(
                                    "REPO get_by_path_impl: skipping key with invalid revision: {}",
                                    e
                                );
                                continue;
                            }
                        };

                        if &revision > max_rev {
                            tracing::debug!(
                                "REPO get_by_path_impl: skipping entry newer than max_revision ({} > {})",
                                revision,
                                max_rev
                            );
                            continue;
                        }
                    }

                    // This is the relevant entry (newest at or before max_revision)
                    // Check if it's a tombstone (path was deleted or moved at this point)
                    if is_tombstone(&bytes) {
                        tracing::info!(
                            "REPO get_by_path_impl: relevant entry is tombstone, path is deleted/moved"
                        );
                        return Ok(None);
                    }

                    let node_id_str = String::from_utf8_lossy(&bytes).to_string();
                    tracing::info!("REPO get_by_path_impl: found node_id={}", node_id_str);
                    break node_id_str;
                }
                Some(Err(e)) => return Err(raisin_error::Error::storage(e.to_string())),
                None => {
                    tracing::info!("REPO get_by_path_impl: no entries found for path");
                    return Ok(None);
                }
            }
        };

        // Public API - populate has_children for frontend display
        match max_revision {
            Some(rev) => {
                self.get_at_revision_impl(
                    tenant_id, repo_id, branch, workspace, &node_id, rev, true,
                )
                .await
            }
            None => {
                self.get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                    .await
            }
        }
    }

    /// Get node ID by path using PATH_INDEX without loading the full node
    pub(crate) async fn get_node_id_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        // Build prefix for this path across all revisions
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
        let mut iter = self.db.prefix_iterator_cf(cf_path, prefix);

        // MVCC semantics with time-travel support:
        // Keys are sorted by revision descending (newest first)
        // For HEAD queries (max_revision = None): first entry is current state
        // For time-travel queries: skip entries newer than max_revision
        // If the relevant entry is a tombstone → path is deleted → return None
        // If the relevant entry is node_id → path exists → return that node
        loop {
            match iter.next() {
                Some(Ok((key, bytes))) => {
                    // Verify key actually starts with our prefix
                    if !key.starts_with(&prefix_clone) {
                        return Ok(None);
                    }

                    // For time-travel queries, skip entries newer than max_revision
                    if let Some(max_rev) = max_revision {
                        let revision = match keys::extract_revision_from_key(&key) {
                            Ok(rev) => rev,
                            Err(_) => continue,
                        };

                        if &revision > max_rev {
                            continue;
                        }
                    }

                    // This is the relevant entry (newest at or before max_revision)
                    // Check if it's a tombstone (path was deleted or moved at this point)
                    if is_tombstone(&bytes) {
                        return Ok(None);
                    }

                    let node_id_str = String::from_utf8_lossy(&bytes).to_string();
                    return Ok(Some(node_id_str));
                }
                Some(Err(e)) => return Err(raisin_error::Error::storage(e.to_string())),
                None => return Ok(None),
            }
        }
    }

    /// Delete node by path
    pub(in crate::repositories::nodes) async fn delete_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<bool> {
        // Always use HEAD for delete operations (no max_revision)
        let node = match self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, path, None)
            .await?
        {
            Some(n) => n,
            None => return Ok(false),
        };

        self.delete_impl(tenant_id, repo_id, branch, workspace, &node.id)
            .await
    }
}
