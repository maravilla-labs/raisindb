//! Deep query operations for hierarchical node retrieval
//!
//! This module provides functions for deep queries that traverse the node tree:
//! - Nested structure (HashMap of DeepNodes)
//! - Flat structure (Vec<Node> in fractional index order)
//! - Array structure (Vec<NodeWithChildren>)

mod array;
mod flat;
mod nested;

use super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;

impl NodeRepositoryImpl {
    /// Resolve the parent ID for deep query operations.
    ///
    /// For root path, returns "/" (root-level children are indexed with parent_id="/").
    /// For non-root, looks up the parent node to get its ID.
    pub(in crate::repositories::nodes) async fn resolve_parent_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<String> {
        if parent_path == "/" || parent_path.is_empty() {
            Ok("/".to_string())
        } else {
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
                .ok_or_else(|| {
                    raisin_error::Error::NotFound("Parent node not found".to_string())
                })?;
            Ok(parent.id)
        }
    }
}
