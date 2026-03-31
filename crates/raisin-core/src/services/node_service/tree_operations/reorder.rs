//! Child reordering operations.
//!
//! Provides methods for reordering children within a parent node using
//! fractional indexing for position management.

use raisin_error::Result;
use raisin_storage::{NodeRepository, Storage};

use super::super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Reorders a child to a specific position in the parent's children list
    pub async fn reorder_child(
        &self,
        parent_path: &str,
        child_name: &str,
        new_position: usize,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // Use fractional indexing for ALL reorder operations (including root level)
        // The storage layer will fetch the ROOT node and use its ID for root-level children
        self.storage
            .nodes()
            .reorder_child(
                self.scope(),
                parent_path,
                child_name,
                new_position,
                message,
                actor,
            )
            .await
    }

    /// Moves a child to appear before another sibling
    pub async fn move_child_before(
        &self,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // Use fractional indexing for ALL reorder operations (including root level)
        // This replaces the old ROOT node children array manipulation
        self.storage
            .nodes()
            .move_child_before(
                self.scope(),
                parent_path,
                child_name,
                before_child_name,
                message,
                actor,
            )
            .await
    }

    /// Moves a child to appear after another sibling
    pub async fn move_child_after(
        &self,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // Use fractional indexing for ALL reorder operations (including root level)
        // The storage layer will fetch the ROOT node and use its ID for root-level children
        self.storage
            .nodes()
            .move_child_after(
                self.scope(),
                parent_path,
                child_name,
                after_child_name,
                message,
                actor,
            )
            .await
    }

    /// Reorders a node relative to a target sibling node
    ///
    /// This high-level method encapsulates all validation logic for reordering.
    /// It ensures both nodes are siblings under the same parent before reordering.
    ///
    /// # Arguments
    /// * `node_path` - Path of the node to reorder
    /// * `target_path` - Path of the target sibling node
    /// * `position` - Either "before" or "after"
    /// * `message` - Optional commit message
    /// * `actor` - Optional actor performing the operation
    ///
    /// # Example
    /// ```ignore
    /// // Move "/folder/item1" to appear before "/folder/item2"
    /// service.reorder("/folder/item1", "/folder/item2", "before", None, None).await?;
    /// ```
    pub async fn reorder(
        &self,
        node_path: &str,
        target_path: &str,
        position: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // Get current node to derive parent and name
        let current_node = self
            .get_by_path(node_path)
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        // Derive parent PATH from node's path (node.parent is the parent NAME, not PATH)
        let parent_path = current_node
            .parent_path()
            .unwrap_or_else(|| "/".to_string());
        let current_name = current_node.name.clone();

        // Derive target sibling name from path
        let target_name = target_path.rsplit('/').next().unwrap_or("");

        // For non-root parent, enforce that target exists under the same parent
        if parent_path != "/" && !parent_path.is_empty() {
            let siblings = self.list_children(&parent_path).await?;
            if !siblings.iter().any(|n| n.name == target_name) {
                return Err(raisin_error::Error::NotFound("target sibling".into()));
            }
        }

        // Perform reorder based on position
        match position {
            "before" => {
                self.move_child_before(&parent_path, &current_name, target_name, message, actor)
                    .await
            }
            "after" => {
                self.move_child_after(&parent_path, &current_name, target_name, message, actor)
                    .await
            }
            _ => Err(raisin_error::Error::Validation(format!(
                "Invalid position '{}', must be 'before' or 'after'",
                position
            ))),
        }
    }
}
