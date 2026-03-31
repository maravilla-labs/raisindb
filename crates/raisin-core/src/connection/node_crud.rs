//! Node CRUD operations for NodeServiceBuilder.

use raisin_error::Result;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope, UpdateNodeOptions};

use super::workspace::NodeServiceBuilder;

impl<'w, S: Storage> NodeServiceBuilder<'w, S> {
    /// Retrieve a node by ID.
    ///
    /// # Example
    /// ```rust,ignore
    /// let node = workspace.nodes().get("node-123").await?;
    /// ```
    pub async fn get(&self, id: &str) -> Result<Option<raisin_models::nodes::Node>> {
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
            .get(scope, id, self.revision.as_ref())
            .await
    }

    /// Create or update a node.
    ///
    /// This performs validation against the node's NodeType schema.
    ///
    /// # Example
    /// ```rust,ignore
    /// workspace.put(node).await?;
    /// ```
    pub async fn put(&self, node: raisin_models::nodes::Node) -> Result<()> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        let node_repo = self.workspace.repository().storage().nodes();
        let exists = node_repo.get(scope, &node.id, None).await?.is_some();

        if exists {
            node_repo
                .update(scope, node, UpdateNodeOptions::default())
                .await
        } else {
            node_repo
                .create(scope, node, CreateNodeOptions::default())
                .await
        }
    }

    /// Delete a node by ID.
    ///
    /// # Example
    /// ```rust,ignore
    /// workspace.nodes().delete("node-123").await?;
    /// ```
    pub async fn delete(&self, id: &str) -> Result<bool> {
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
            .delete(scope, id, raisin_storage::DeleteNodeOptions::default())
            .await
    }

    /// Get a node by its path.
    ///
    /// # Example
    /// ```rust,ignore
    /// let node = workspace.nodes().get_by_path("/content/homepage").await?;
    /// ```
    pub async fn get_by_path(&self, path: &str) -> Result<Option<raisin_models::nodes::Node>> {
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
            .get_by_path(scope, path, None)
            .await
    }

    /// List all nodes in workspace (DEPRECATED - use list_root() instead).
    ///
    /// This method is deprecated because it doesn't handle nested tree structures properly.
    /// Use `list_root()` for root-level nodes or connection API methods with level/depth parameters.
    #[deprecated(
        since = "0.1.0",
        note = "Use list_root() or deep_children methods instead"
    )]
    pub async fn list_all(&self) -> Result<Vec<raisin_models::nodes::Node>> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        let options = if let Some(rev) = &self.revision {
            raisin_storage::ListOptions::at_revision(*rev)
        } else {
            raisin_storage::ListOptions::for_sql()
        };

        self.workspace
            .repository()
            .storage()
            .nodes()
            .list_all(scope, options)
            .await
    }

    /// List nodes by NodeType.
    pub async fn list_by_type(&self, node_type: &str) -> Result<Vec<raisin_models::nodes::Node>> {
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
            .list_by_type(scope, node_type, raisin_storage::ListOptions::for_sql())
            .await
    }

    /// List nodes by parent ID.
    pub async fn list_by_parent(&self, parent: &str) -> Result<Vec<raisin_models::nodes::Node>> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        let options = if let Some(rev) = &self.revision {
            raisin_storage::ListOptions::at_revision(*rev)
        } else {
            raisin_storage::ListOptions::for_sql()
        };

        self.workspace
            .repository()
            .storage()
            .nodes()
            .list_by_parent(scope, parent, options)
            .await
    }

    /// List root-level nodes.
    pub async fn list_root(&self) -> Result<Vec<raisin_models::nodes::Node>> {
        let scope = StorageScope::new(
            self.repository().tenant_id(),
            self.repository().repo_id(),
            self.effective_branch(),
            self.workspace.workspace_id(),
        );

        let options = if let Some(rev) = &self.revision {
            raisin_storage::ListOptions::at_revision(*rev)
        } else {
            raisin_storage::ListOptions::for_sql()
        };

        self.workspace
            .repository()
            .storage()
            .nodes()
            .list_root(scope, options)
            .await
    }
}
