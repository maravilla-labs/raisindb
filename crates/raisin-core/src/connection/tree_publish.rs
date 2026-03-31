//! Tree operations, publishing, and transaction support for NodeServiceBuilder.

use raisin_error::Result;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::workspace::NodeServiceBuilder;

impl<'w, S: Storage> NodeServiceBuilder<'w, S> {
    // ========================================================================
    // Tree Operations
    // ========================================================================

    /// Copy a single node to a new location.
    ///
    /// Does not copy children - use `copy_node_tree` for recursive copy.
    pub async fn copy_node(
        &self,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
    ) -> Result<raisin_models::nodes::Node> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        self.workspace
            .repository()
            .storage()
            .nodes()
            .copy_node(scope, source_path, target_parent, new_name, None)
            .await
    }

    /// Recursively copy a node and all its descendants.
    pub async fn copy_node_tree(
        &self,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
    ) -> Result<raisin_models::nodes::Node> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        self.workspace
            .repository()
            .storage()
            .nodes()
            .copy_node_tree(scope, source_path, target_parent, new_name, None)
            .await
    }

    /// Get deep children flattened (for tree operations).
    pub async fn deep_children_flat(
        &self,
        node_path: &str,
        max_depth: u32,
    ) -> Result<Vec<raisin_models::nodes::Node>> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        self.workspace
            .repository()
            .storage()
            .nodes()
            .deep_children_flat(scope, node_path, max_depth, None)
            .await
    }

    // ========================================================================
    // Publishing Operations
    // ========================================================================

    /// Publish a single node.
    pub async fn publish(&self, node_path: &str) -> Result<()> {
        let mut node = self
            .get_by_path(node_path)
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        node.published_at = Some(chrono::Utc::now());
        node.published_by = Some("system".into());
        self.put(node).await?;

        Ok(())
    }

    /// Publish a node and all its descendants.
    pub async fn publish_tree(&self, node_path: &str) -> Result<()> {
        self.publish(node_path).await?;

        let descendants = self.deep_children_flat(node_path, 100).await?;
        for mut node in descendants {
            node.published_at = Some(chrono::Utc::now());
            node.published_by = Some("system".into());
            self.put(node).await?;
        }

        Ok(())
    }

    /// Unpublish a single node.
    pub async fn unpublish(&self, node_path: &str) -> Result<()> {
        let mut node = self
            .get_by_path(node_path)
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        node.published_at = None;
        node.published_by = None;
        self.put(node).await?;

        Ok(())
    }

    /// Unpublish a node and all its descendants.
    pub async fn unpublish_tree(&self, node_path: &str) -> Result<()> {
        self.unpublish(node_path).await?;

        let descendants = self.deep_children_flat(node_path, 100).await?;
        for mut node in descendants {
            node.published_at = None;
            node.published_by = None;
            self.put(node).await?;
        }

        Ok(())
    }

    // ========================================================================
    // Transaction API
    // ========================================================================

    /// Create a new transaction for atomic multi-node operations.
    ///
    /// Accumulates operations and commits them all at once, creating a single
    /// repository revision.
    ///
    /// # Example
    /// ```rust,ignore
    /// let mut tx = workspace.nodes().transaction();
    /// tx.create(node1);
    /// tx.update(node2_id, props);
    /// tx.delete(node3_id);
    /// tx.commit("Bulk update", "user-123").await?;
    /// ```
    pub fn transaction(&self) -> crate::Transaction<S> {
        let tenant_id = self.repository().tenant_id().to_string();
        let repo_id = self.repository().repo_id().to_string();
        let branch = self.effective_branch().to_string();
        let workspace_id = self.workspace.workspace_id().to_string();

        crate::Transaction::new(
            self.workspace.repository().storage().clone(),
            tenant_id,
            repo_id,
            branch,
            workspace_id,
        )
    }
}
