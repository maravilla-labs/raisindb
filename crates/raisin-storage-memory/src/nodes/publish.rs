use raisin_error::Result;
use raisin_models as models;
use raisin_storage::scope::StorageScope;
use raisin_storage::{PropertyIndexRepository, ReferenceIndexRepository};

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Marks a node as published by setting its published_at timestamp.
pub(super) async fn publish(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let (node_id, properties) = {
        let mut map = repo.nodes.write().await;
        if let Some((key, _)) = map
            .iter()
            .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == node_path)
            .map(|(k, n)| (k.clone(), n.clone()))
        {
            if let Some(node) = map.get_mut(&key) {
                if node.published_at.is_none() {
                    node.published_at = Some(chrono::Utc::now());
                    node.updated_at = Some(chrono::Utc::now());
                }
                (node.id.clone(), node.properties.clone())
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }; // Release lock before indexing

    // Update property index status: draft -> published
    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
    repo.property_index
        .update_publish_status(scope, &node_id, &properties, true)
        .await?;

    // Update reference index status: draft -> published
    let dummy_revision = raisin_hlc::HLC::new(0, 0);
    repo.reference_index
        .update_reference_publish_status(scope, &node_id, &properties, &dummy_revision, true)
        .await?;

    Ok(())
}

/// Publishes a node and all its descendants.
pub(super) async fn publish_tree(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    // Publish the root node
    publish(repo, tenant_id, repo_id, branch, workspace, node_path).await?;

    // Find all descendants and publish them
    let map = repo.nodes.read().await;
    let descendants: Vec<String> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix) && n.path.starts_with(&(node_path.to_string() + "/"))
        })
        .map(|(_, n)| n.path.clone())
        .collect();
    drop(map);

    for desc_path in descendants {
        publish(repo, tenant_id, repo_id, branch, workspace, &desc_path).await?;
    }

    Ok(())
}

/// Unpublishes a node by clearing its published_at timestamp.
pub(super) async fn unpublish(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let (node_id, properties) = {
        let mut map = repo.nodes.write().await;
        if let Some((key, _)) = map
            .iter()
            .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == node_path)
            .map(|(k, n)| (k.clone(), n.clone()))
        {
            if let Some(node) = map.get_mut(&key) {
                node.published_at = None;
                node.updated_at = Some(chrono::Utc::now());
                (node.id.clone(), node.properties.clone())
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }; // Release lock before indexing

    // Update property index status: published -> draft
    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
    repo.property_index
        .update_publish_status(scope, &node_id, &properties, false)
        .await?;

    // Update reference index status: published -> draft
    let dummy_revision = raisin_hlc::HLC::new(0, 0);
    repo.reference_index
        .update_reference_publish_status(scope, &node_id, &properties, &dummy_revision, false)
        .await?;

    Ok(())
}

/// Unpublishes a node and all its descendants.
pub(super) async fn unpublish_tree(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    // Unpublish the root node
    unpublish(repo, tenant_id, repo_id, branch, workspace, node_path).await?;

    // Find all descendants and unpublish them
    let map = repo.nodes.read().await;
    let descendants: Vec<String> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix) && n.path.starts_with(&(node_path.to_string() + "/"))
        })
        .map(|(_, n)| n.path.clone())
        .collect();
    drop(map);

    for desc_path in descendants {
        unpublish(repo, tenant_id, repo_id, branch, workspace, &desc_path).await?;
    }

    Ok(())
}

/// Retrieves a published node by its ID.
///
/// Returns None if the node is not found or not published.
pub(super) async fn get_published(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    id: &str,
) -> Result<Option<models::nodes::Node>> {
    let key = NodeKey::new(tenant_id, repo_id, branch, workspace, id).to_path();

    let map = repo.nodes.read().await;
    if let Some(node) = map.get(&key).cloned() {
        if node.published_at.is_some() {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

/// Retrieves a published node by its path.
///
/// Returns None if the node is not found or not published.
pub(super) async fn get_published_by_path(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<Option<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let map = repo.nodes.read().await;
    for (k, node) in map.iter() {
        if k.starts_with(&workspace_prefix) && node.path == path && node.published_at.is_some() {
            return Ok(Some(node.clone()));
        }
    }
    Ok(None)
}

/// Lists all published direct children of a parent node.
pub(super) async fn list_published_children(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let map = repo.nodes.read().await;
    let mut out: Vec<_> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix)
                && n.parent_path().as_deref() == Some(parent_path)
                && n.published_at.is_some()
        })
        .map(|(_, n)| n.clone())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Lists all published root nodes (nodes without a parent).
pub(super) async fn list_published_root(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let map = repo.nodes.read().await;
    let mut out: Vec<_> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix)
                && (n.parent.is_none() || n.parent.as_deref() == Some(""))
                && n.published_at.is_some()
        })
        .map(|(_, n)| n.clone())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
