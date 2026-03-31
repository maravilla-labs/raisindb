//! In-process API facade for RaisinDB

use raisin_core::{NodeService, WorkspaceService};
use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::sync::Arc;

pub struct InProcessApi<S: Storage + TransactionalStorage> {
    storage: Arc<S>,
    ws_svc: Arc<WorkspaceService<S>>,
}

impl<S: Storage + TransactionalStorage> InProcessApi<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage: storage.clone(),
            ws_svc: Arc::new(WorkspaceService::new(storage)),
        }
    }

    /// Create a scoped NodeService for the given context
    /// Uses default tenant/repo/branch for now
    fn node_service(&self, workspace: &str) -> NodeService<S> {
        NodeService::new_with_context(
            self.storage.clone(),
            "default".to_string(), // tenant_id
            "main".to_string(),    // repo_id
            "main".to_string(),    // branch
            workspace.to_string(),
        )
    }

    // Workspace
    pub async fn list_workspaces(&self, repo: &str) -> Result<Vec<models::workspace::Workspace>> {
        self.ws_svc.list("default", repo).await
    }
    pub async fn get_workspace(
        &self,
        repo: &str,
        name: &str,
    ) -> Result<Option<models::workspace::Workspace>> {
        self.ws_svc.get("default", repo, name).await
    }
    pub async fn put_workspace(&self, repo: &str, ws: models::workspace::Workspace) -> Result<()> {
        self.ws_svc.put("default", repo, ws).await
    }

    // Nodes
    pub async fn get_node(&self, ws: &str, id: &str) -> Result<Option<models::nodes::Node>> {
        self.node_service(ws).get(id).await
    }
    pub async fn put_node(&self, ws: &str, node: models::nodes::Node) -> Result<()> {
        self.node_service(ws).upsert(node).await
    }
    pub async fn delete_node(&self, ws: &str, id: &str) -> Result<bool> {
        self.node_service(ws).delete(id).await
    }
    pub async fn get_by_path(&self, ws: &str, path: &str) -> Result<Option<models::nodes::Node>> {
        self.node_service(ws).get_by_path(path).await
    }
    pub async fn list_all(&self, ws: &str) -> Result<Vec<models::nodes::Node>> {
        self.node_service(ws).list_root().await
    }
    pub async fn list_root(&self, ws: &str) -> Result<Vec<models::nodes::Node>> {
        self.node_service(ws).list_root().await
    }
    pub async fn list_children(&self, ws: &str, parent: &str) -> Result<Vec<models::nodes::Node>> {
        self.node_service(ws).list_children(parent).await
    }
    pub async fn move_node(&self, ws: &str, id: &str, new_path: &str) -> Result<()> {
        self.node_service(ws).move_node(id, new_path).await
    }
    pub async fn rename_node(&self, ws: &str, old_path: &str, new_name: &str) -> Result<()> {
        self.node_service(ws).rename_node(old_path, new_name).await
    }
    pub async fn delete_by_path(&self, ws: &str, path: &str) -> Result<bool> {
        self.node_service(ws).delete_by_path(path).await
    }
    pub async fn deep_children_nested(
        &self,
        ws: &str,
        parent: &str,
        max_depth: u32,
    ) -> Result<std::collections::HashMap<String, models::nodes::DeepNode>> {
        self.node_service(ws)
            .deep_children_nested(parent, max_depth)
            .await
    }
    pub async fn deep_children_flat(
        &self,
        ws: &str,
        parent: &str,
        max_depth: u32,
    ) -> Result<Vec<models::nodes::Node>> {
        self.node_service(ws)
            .deep_children_flat(parent, max_depth)
            .await
    }
    pub async fn reorder_child(
        &self,
        ws: &str,
        parent: &str,
        child: &str,
        pos: usize,
    ) -> Result<()> {
        self.node_service(ws)
            .reorder_child(parent, child, pos, None, None)
            .await
    }
    pub async fn move_child_before(
        &self,
        ws: &str,
        parent: &str,
        child: &str,
        before: &str,
    ) -> Result<()> {
        self.node_service(ws)
            .move_child_before(parent, child, before, None, None)
            .await
    }
    pub async fn move_child_after(
        &self,
        ws: &str,
        parent: &str,
        child: &str,
        after: &str,
    ) -> Result<()> {
        self.node_service(ws)
            .move_child_after(parent, child, after, None, None)
            .await
    }
}
