//! Node listing operations
//!
//! This module provides functions for listing nodes by various criteria:
//! - List by type
//! - List by parent
//! - List root nodes
//! - List children
//! - Check if node has children

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// List nodes by type using __node_type pseudo-property index
    pub(in crate::repositories::nodes) async fn list_by_type_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // Use __node_type pseudo-property index for efficient lookup
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("prop") // Non-published properties
            .push("__node_type")
            .push(node_type)
            .build_prefix();

        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_property, prefix);

        let mut node_ids = std::collections::HashSet::new();

        // Collect unique node IDs (deduplicate across revisions)
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Skip tombstones
            if is_tombstone(&value) {
                continue;
            }

            // Extract node_id from key (last component)
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if let Some(node_id_bytes) = parts.last() {
                let node_id = String::from_utf8_lossy(node_id_bytes).to_string();
                node_ids.insert(node_id);
            }
        }

        // Fetch actual nodes
        let mut nodes = Vec::new();
        for node_id in node_ids {
            // Public API - populate has_children for frontend display
            let node_opt = match max_revision {
                Some(rev) => {
                    self.get_at_revision_impl(
                        tenant_id, repo_id, branch, workspace, &node_id, rev, true,
                    )
                    .await?
                }
                None => {
                    self.get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                        .await?
                }
            };
            if let Some(node) = node_opt {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    /// List children using ORDERED_CHILDREN index
    pub(in crate::repositories::nodes) async fn list_by_parent_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent: &str,
        max_revision: Option<&HLC>,
        populate_has_children: bool,
    ) -> Result<Vec<Node>> {
        // Special case: "/" is the parent NAME for root-level nodes,
        // For root nodes, we use "/" itself as the parent_id
        let parent_id = if parent == "/" {
            "/".to_string()
        } else {
            // For non-root parents, parent is already the ID
            parent.to_string()
        };

        tracing::debug!(
            "list_by_parent_impl: tenant={}, repo={}, branch={}, workspace={}, parent={}, max_revision={:?}",
            tenant_id, repo_id, branch, workspace, parent_id, max_revision
        );

        // Use ORDERED_CHILDREN index for efficient ordered retrieval
        let child_ids = self
            .get_ordered_child_ids(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &parent_id,
                max_revision,
            )
            .await?;

        tracing::debug!(
            "list_by_parent_impl: got {} child IDs from ordered index",
            child_ids.len()
        );

        // Fetch nodes in order
        let mut result = Vec::with_capacity(child_ids.len());
        for child_id in child_ids {
            // Pass through the populate_has_children parameter
            let node_opt = match max_revision {
                Some(rev) => {
                    self.get_at_revision_impl(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &child_id,
                        rev,
                        populate_has_children,
                    )
                    .await?
                }
                None => {
                    self.get_impl(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &child_id,
                        populate_has_children,
                    )
                    .await?
                }
            };
            if let Some(node) = node_opt {
                result.push(node);
            }
        }

        Ok(result)
    }

    /// List root nodes (nodes whose parent is "/" or root itself)
    pub(in crate::repositories::nodes) async fn list_root_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // Public API - populate has_children for frontend display
        self.list_by_parent_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "/",
            max_revision,
            true,
        )
        .await
    }

    /// List children by parent path
    pub(in crate::repositories::nodes) async fn list_children_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // Special case: root path "/" uses "/" as parent_id in ORDERED_CHILDREN index
        // This matches the logic in add_impl/update_impl where root-level nodes are indexed with parent_id = "/"
        if parent_path == "/" {
            return self
                .list_by_parent_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "/",
                    max_revision,
                    true,
                )
                .await;
        }

        let parent = self
            .get_by_path_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_path,
                max_revision,
            )
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Parent node not found".to_string()))?;

        // Public API - populate has_children for frontend display
        self.list_by_parent_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent.id,
            max_revision,
            true,
        )
        .await
    }

    /// Check if node has children
    ///
    /// This is an optimized check that only scans the ORDERED_CHILDREN index
    /// to see if any children exist, without fetching full node data.
    pub(in crate::repositories::nodes) async fn has_children_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        // Special case: ROOT node's children are indexed under "/" not the ROOT node's actual ID
        // Check if this is the ROOT node by looking it up
        let parent_id_for_lookup = if let Some(node) = self
            .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
            .await?
        {
            if node.path == "/" {
                "/" // ROOT node's children are indexed under "/"
            } else {
                node_id
            }
        } else {
            node_id
        };

        // Just check if there are any child IDs in the ordered index
        // This is much more efficient than fetching all children
        let child_ids = self
            .get_ordered_child_ids(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id_for_lookup,
                max_revision,
            )
            .await?;
        Ok(!child_ids.is_empty())
    }
}
