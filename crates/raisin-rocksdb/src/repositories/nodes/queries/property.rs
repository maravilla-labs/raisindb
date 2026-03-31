//! Node property query and update operations
//!
//! This module provides functions for querying and updating node properties.

use super::super::helpers::{hash_property_value, is_tombstone};
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Find nodes by property name and value
    pub(in crate::repositories::nodes) async fn find_by_property_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        property_value: &PropertyValue,
    ) -> Result<Vec<Node>> {
        // Use PROPERTY_INDEX for efficient lookup
        let value_hash = hash_property_value(property_value);

        // Build prefix for this property+value across all revisions
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("prop") // Non-published properties
            .push(property_name)
            .push(&value_hash)
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
            if let Some(node) = self
                .get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                .await?
            {
                // Double-check property value matches (hash collisions are possible)
                if node.properties.get(property_name) == Some(property_value) {
                    nodes.push(node);
                }
            }
        }

        Ok(nodes)
    }

    /// Find nodes with a specific property (any value)
    pub(in crate::repositories::nodes) async fn find_nodes_with_property_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
    ) -> Result<Vec<Node>> {
        // Use PROPERTY_INDEX for efficient lookup (prefix scan without value_hash)
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("prop") // Non-published properties
            .push(property_name)
            .build_prefix();

        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_property, prefix);

        let mut node_ids = std::collections::HashSet::new();

        // Collect unique node IDs (deduplicate across revisions and values)
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
            if let Some(node) = self
                .get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                .await?
            {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    /// Get property value by path
    pub(in crate::repositories::nodes) async fn get_property_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<PropertyValue>> {
        let node = self
            .get_by_path_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_path,
                max_revision,
            )
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        Ok(node.properties.get(property_path).cloned())
    }

    /// Update property by path
    pub(in crate::repositories::nodes) async fn update_property_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: PropertyValue,
    ) -> Result<()> {
        // Always use HEAD for write operations (no max_revision)
        let mut node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        node.properties.insert(property_path.to_string(), value);

        self.update_impl(tenant_id, repo_id, branch, workspace, node)
            .await
    }
}
