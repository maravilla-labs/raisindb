//! Publishing and unpublishing operations

use super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_models::nodes::Node;
use std::future::Future;
use std::pin::Pin;

impl NodeRepositoryImpl {
    /// Publish a node
    pub(super) async fn publish_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
    ) -> Result<()> {
        let mut node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        node.published_at = Some(chrono::Utc::now());

        self.update_impl(tenant_id, repo_id, branch, workspace, node)
            .await
    }

    /// Publish a node tree recursively
    ///
    /// Publishes a node and all its descendants in a depth-first traversal.
    /// Sets `published_at` timestamp for each node in the tree.
    ///
    /// # Arguments
    /// * `node_path` - Path to the root node of the tree to publish
    pub(super) async fn publish_tree_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
    ) -> Result<()> {
        // Get the root node
        let node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        // Recursively publish this node and all descendants
        self.publish_tree_recursive(tenant_id, repo_id, branch, workspace, &node.id)
            .await
    }

    /// Recursive helper for publishing a node tree
    ///
    /// # Recursion Limit
    ///
    /// This implementation uses async recursion which consumes stack space.
    /// Safe recursion depth is approximately 100-200 levels depending on the platform.
    /// For very deep trees (>100 levels), the operation may fail with a stack overflow.
    /// Consider publishing in batches for extremely deep trees.
    fn publish_tree_recursive<'a>(
        &'a self,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
        node_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Get the current node (internal operation - no need to populate has_children)
            let mut node = match self
                .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
                .await?
            {
                Some(n) => n,
                None => {
                    tracing::warn!(
                        node_id = %node_id,
                        "Node not found during publish_tree operation - skipping"
                    );
                    return Ok(());
                }
            };

            // Publish this node by setting the published_at timestamp
            node.published_at = Some(chrono::Utc::now());
            self.update_impl(tenant_id, repo_id, branch, workspace, node.clone())
                .await?;

            // Get ordered children (use None - publish operations always work on current HEAD)
            let child_ids = self
                .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, node_id, None)
                .await?;

            // Recursively publish each child
            for child_id in child_ids {
                self.publish_tree_recursive(tenant_id, repo_id, branch, workspace, &child_id)
                    .await?;
            }

            Ok(())
        })
    }

    /// Unpublish a node
    pub(super) async fn unpublish_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
    ) -> Result<()> {
        let mut node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        node.published_at = None;

        self.update_impl(tenant_id, repo_id, branch, workspace, node)
            .await
    }

    /// Unpublish a node tree recursively
    ///
    /// Unpublishes a node and all its descendants in a depth-first traversal.
    /// Clears the `published_at` timestamp for each node in the tree.
    ///
    /// # Arguments
    /// * `node_path` - Path to the root node of the tree to unpublish
    pub(super) async fn unpublish_tree_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
    ) -> Result<()> {
        // Get the root node
        let node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        // Recursively unpublish this node and all descendants
        self.unpublish_tree_recursive(tenant_id, repo_id, branch, workspace, &node.id)
            .await
    }

    /// Recursive helper for unpublishing a node tree
    ///
    /// # Recursion Limit
    ///
    /// This implementation uses async recursion which consumes stack space.
    /// Safe recursion depth is approximately 100-200 levels depending on the platform.
    /// For very deep trees (>100 levels), the operation may fail with a stack overflow.
    /// Consider unpublishing in batches for extremely deep trees.
    fn unpublish_tree_recursive<'a>(
        &'a self,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
        node_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Get the current node (internal operation - no need to populate has_children)
            let mut node = match self
                .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
                .await?
            {
                Some(n) => n,
                None => {
                    tracing::warn!(
                        node_id = %node_id,
                        "Node not found during unpublish_tree operation - skipping"
                    );
                    return Ok(());
                }
            };

            // Unpublish this node by clearing the published_at timestamp
            node.published_at = None;
            self.update_impl(tenant_id, repo_id, branch, workspace, node.clone())
                .await?;

            // Get ordered children (use None - unpublish operations always work on current HEAD)
            let child_ids = self
                .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, node_id, None)
                .await?;

            // Recursively unpublish each child
            for child_id in child_ids {
                self.unpublish_tree_recursive(tenant_id, repo_id, branch, workspace, &child_id)
                    .await?;
            }

            Ok(())
        })
    }

    /// Get published node by ID
    pub(super) async fn get_published_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
    ) -> Result<Option<Node>> {
        // Public API - populate has_children for frontend display
        let node = self
            .get_impl(tenant_id, repo_id, branch, workspace, id, true)
            .await?;

        Ok(node.filter(|n| n.published_at.is_some()))
    }

    /// Get published node by path
    pub(super) async fn get_published_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Node>> {
        let node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, path, None)
            .await?;

        Ok(node.filter(|n| n.published_at.is_some()))
    }

    /// List published children
    pub(super) async fn list_published_children_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Node>> {
        let children = self
            .list_children_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
            .await?;

        Ok(children
            .into_iter()
            .filter(|n| n.published_at.is_some())
            .collect())
    }

    /// List published root nodes
    pub(super) async fn list_published_root_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<Node>> {
        let roots = self
            .list_root_impl(tenant_id, repo_id, branch, workspace, None)
            .await?;

        Ok(roots
            .into_iter()
            .filter(|n| n.published_at.is_some())
            .collect())
    }
}
