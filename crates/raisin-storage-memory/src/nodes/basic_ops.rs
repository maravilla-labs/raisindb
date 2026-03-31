use raisin_error::Result;
use raisin_models as models;
use raisin_storage::scope::StorageScope;
use raisin_storage::{PropertyIndexRepository, ReferenceIndexRepository};

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Retrieves a node by its ID from the specified workspace.
pub(super) async fn get(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    id: &str,
) -> Result<Option<models::nodes::Node>> {
    let key = NodeKey::new(tenant_id, repo_id, branch, workspace, id).to_path();
    let map = repo.nodes.read().await;
    Ok(map.get(&key).cloned())
}

/// Stores a node in the repository and updates the parent's children list if applicable.
pub(super) async fn put(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    mut node: models::nodes::Node,
) -> Result<()> {
    // VALIDATION: Storage layer enforces data integrity
    if node.id.is_empty() {
        return Err(raisin_error::Error::Validation(
            "Node ID cannot be empty".to_string(),
        ));
    }
    if node.path.is_empty() {
        return Err(raisin_error::Error::Validation(
            "Node path cannot be empty".to_string(),
        ));
    }

    let node_id = node.id.clone();
    let node_name = node.name.clone();

    // CRITICAL: Auto-derive parent NAME from path before saving
    // This ensures node.parent always contains the parent's NAME, not PATH
    node.parent = models::nodes::Node::extract_parent_name_from_path(&node.path);

    // Derive parent PATH from node's path for finding parent node
    let parent_path = node.parent_path();
    let is_published = node.published_at.is_some();
    let properties = node.properties.clone();

    {
        let key = NodeKey::new(tenant_id, repo_id, branch, workspace, &node_id).to_path();
        let mut map = repo.nodes.write().await;
        map.insert(key, node);

        // Update parent's children list using parent PATH
        if let Some(pp) = parent_path.as_deref() {
            // Find parent node by path within same workspace/branch
            let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
            if let Some((parent_key, _)) = map
                .iter()
                .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == pp)
                .map(|(k, n)| (k.clone(), n.clone()))
            {
                if let Some(parent_node) = map.get_mut(&parent_key) {
                    if !parent_node.children.iter().any(|c| c == &node_name) {
                        parent_node.children.push(node_name.clone());
                    }
                }
            }
        }
    } // Release lock before indexing

    // Update property index
    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
    repo.property_index
        .index_properties(scope, &node_id, &properties, is_published)
        .await?;

    // Update reference index
    let dummy_revision = raisin_hlc::HLC::new(0, 0);
    repo.reference_index
        .index_references(scope, &node_id, &properties, &dummy_revision, is_published)
        .await?;

    Ok(())
}

/// Deletes a node by its ID from the specified workspace.
pub(super) async fn delete(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    id: &str,
) -> Result<bool> {
    // Get node properties before deletion for unindexing
    let properties = {
        let key = NodeKey::new(tenant_id, repo_id, branch, workspace, id).to_path();
        let map = repo.nodes.read().await;
        map.get(&key).map(|n| n.properties.clone())
    };

    // First, unindex properties
    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
    repo.property_index.unindex_properties(scope, id).await?;

    // Unindex references if node exists
    if let Some(props) = properties {
        let dummy_revision = raisin_hlc::HLC::new(0, 0);
        repo.reference_index
            .unindex_references(scope, id, &props, &dummy_revision)
            .await?;
    }

    // Then, delete the node
    let key = NodeKey::new(tenant_id, repo_id, branch, workspace, id).to_path();
    let mut map = repo.nodes.write().await;
    Ok(map.remove(&key).is_some())
}

/// Check if a node has children
///
/// This is more efficient than loading all children just to check if any exist.
/// Used to populate the `has_children` field in JSON responses.
pub(super) async fn has_children(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<bool> {
    let key = NodeKey::new(tenant_id, repo_id, branch, workspace, node_id).to_path();
    let map = repo.nodes.read().await;

    // Check if the node exists and has a non-empty children array
    if let Some(node) = map.get(&key) {
        Ok(!node.children.is_empty())
    } else {
        // Node doesn't exist - return false
        Ok(false)
    }
}

/// Lists all nodes of a specific type in the workspace.
pub(super) async fn list_by_type(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;
    let out = map
        .iter()
        .filter(|(k, n)| k.starts_with(&workspace_prefix) && n.node_type == node_type)
        .map(|(_, n)| n.clone())
        .collect();
    Ok(out)
}

/// Lists all direct children of a parent node.
pub(super) async fn list_by_parent(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent: &str,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;

    // Filter children by comparing derived parent PATH (from node.path) with the passed-in parent PATH
    let mut children: Vec<_> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix) && n.parent_path().as_deref() == Some(parent)
        })
        .map(|(_, n)| n.clone())
        .collect();

    // Get parent node to retrieve children order
    if !parent.is_empty() {
        if let Some((_, parent_node)) = map
            .iter()
            .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent)
        {
            let order = &parent_node.children;

            // Sort according to parent's children order
            children.sort_by(|a, b| {
                let ia = order.iter().position(|n| n == &a.name);
                let ib = order.iter().position(|n| n == &b.name);
                match (ia, ib) {
                    (Some(ia), Some(ib)) => ia.cmp(&ib),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            });
        }
    }

    Ok(children)
}

/// Retrieves a node by its path from the specified workspace.
pub(super) async fn get_by_path(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<Option<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;
    let found = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == path)
        .map(|(_, n)| n.clone());
    Ok(found)
}

/// Lists all nodes in the workspace.
pub(super) async fn list_all(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;
    let out = map
        .iter()
        .filter(|(k, _)| k.starts_with(&workspace_prefix))
        .map(|(_, n)| n.clone())
        .collect();
    Ok(out)
}

/// Counts all nodes in the workspace without loading node data.
///
/// This is more memory-efficient than list_all() for COUNT(*) queries
/// since it only counts keys without cloning node data.
pub(super) async fn count_all(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<usize> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;
    let count = map
        .iter()
        .filter(|(k, _)| k.starts_with(&workspace_prefix))
        .count();
    Ok(count)
}
