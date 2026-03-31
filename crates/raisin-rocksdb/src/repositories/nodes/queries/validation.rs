//! Validation helpers for tree operations
//!
//! This module provides validation functions for copy, move, and rename operations.

use super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Validate that a parent node exists and is accessible
    pub(in crate::repositories::nodes) async fn validate_parent_exists(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Node> {
        self.get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Target parent '{}' not found", parent_path))
            })
    }
    /// Validate no circular reference (target is not source or source's descendant)
    pub(in crate::repositories::nodes) async fn validate_no_circular_reference(
        &self,
        source_path: &str,
        target_parent_path: &str,
    ) -> Result<()> {
        // Case 1: Cannot move/copy into itself
        if source_path == target_parent_path {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot move/copy node '{}' into itself",
                source_path
            )));
        }

        // Case 2: Cannot move/copy into own descendant
        // Check if target_parent_path starts with source_path + "/"
        let source_prefix = format!("{}/", source_path);
        if target_parent_path.starts_with(&source_prefix) {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot move/copy node '{}' into its own descendant '{}'",
                source_path, target_parent_path
            )));
        }

        Ok(())
    }

    /// Validate that the new name is unique among the target parent's children
    pub(in crate::repositories::nodes) async fn validate_unique_child_name(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        new_name: &str,
    ) -> Result<()> {
        // Get all children of the target parent
        let child_ids = self
            .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, parent_id, None)
            .await?;

        // Check each child's name
        for child_id in child_ids {
            if let Some(child) = self
                .get_impl(tenant_id, repo_id, branch, workspace, &child_id, false)
                .await?
            {
                if child.name == new_name {
                    return Err(raisin_error::Error::Validation(format!(
                        "A child with name '{}' already exists in the target location",
                        new_name
                    )));
                }
            }
        }

        Ok(())
    }

    /// Validate that the operation is not being performed on the root node
    pub(in crate::repositories::nodes) fn validate_not_root_node(&self, path: &str) -> Result<()> {
        if path == "/" {
            return Err(raisin_error::Error::Validation(
                "Cannot copy or move the root node".to_string(),
            ));
        }
        Ok(())
    }
}
