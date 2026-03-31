//! Tree operations for moving and renaming nodes
//!
//! This module provides functions for:
//! - Moving single nodes
//! - Moving entire node trees
//! - Renaming nodes
//!
//! # StorageNode Optimization
//!
//! Since nodes are stored as StorageNode (without path), move operations
//! only need to update indexes, not rewrite node blobs. This gives O(1)
//! cost per node for path changes (vs O(N) with embedded paths).
//!
//! Move operations update:
//! - PATH_INDEX: tombstone old path, write new path -> node_id
//! - NODE_PATH: write node_id -> new path
//! - ORDERED_CHILDREN: only if parent changes (root node only for tree moves)

mod move_node;
mod move_tree;

use super::super::NodeRepositoryImpl;
use raisin_error::Result;

impl NodeRepositoryImpl {
    /// Rename node
    pub(in crate::repositories::nodes) async fn rename_node_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        old_path: &str,
        new_name: &str,
    ) -> Result<()> {
        let parent_path = old_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
        let new_path = if parent_path.is_empty() {
            format!("/{}", new_name)
        } else {
            format!("{}/{}", parent_path, new_name)
        };

        let node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, old_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        // TODO: Create OperationMeta::Rename for audit trail
        self.move_node_impl(
            tenant_id, repo_id, branch, workspace, &node.id, &new_path, None,
        )
        .await
    }
}
