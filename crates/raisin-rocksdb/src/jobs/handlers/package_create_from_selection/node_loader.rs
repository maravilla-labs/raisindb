//! Node loading logic for package creation

use super::PackageCreateFromSelectionHandler;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

impl PackageCreateFromSelectionHandler {
    /// Load nodes at a given path, optionally recursively
    pub(super) async fn load_nodes_recursive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        recursive: bool,
    ) -> Result<Vec<Node>> {
        let node_repo = self.storage.nodes();
        let mut nodes = Vec::new();

        // Try to get the node at this path (use get_by_path since we have a path, not an ID)
        if let Some(node) = node_repo
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                path,
                None,
            )
            .await?
        {
            nodes.push(node);
        }

        // If recursive, get children
        if recursive {
            let children = node_repo
                .list_children(
                    StorageScope::new(tenant_id, repo_id, branch, workspace),
                    path,
                    ListOptions::default(),
                )
                .await?;

            for child in children {
                let child_path = child.path.clone();

                // Recursively load this child and its descendants
                let child_nodes = Box::pin(self.load_nodes_recursive(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &child_path,
                    true,
                ))
                .await?;
                nodes.extend(child_nodes);
            }
        }

        Ok(nodes)
    }
}
