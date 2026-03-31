use raisin_error::Result;

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Reorders a child within its parent's children list to a specific position.
pub(super) async fn reorder_child(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    child_name: &str,
    new_position: usize,
    _message: Option<&str>,
    _actor: Option<&str>,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let mut map = repo.nodes.write().await;
    // find parent by path
    let key = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent_path)
        .map(|(k, _)| k.clone());
    if let Some(k) = key {
        if let Some(p) = map.get_mut(&k) {
            if let Some(idx) = p.children.iter().position(|c| c == child_name) {
                p.children.remove(idx);
            }
            let pos = new_position.min(p.children.len());
            p.children.insert(pos, child_name.to_string());
            // Update parent's updated_at timestamp for publish tracking
            p.updated_at = Some(chrono::Utc::now());
        }
    }
    Ok(())
}

/// Moves a child to be positioned immediately before another child.
pub(super) async fn move_child_before(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    child_name: &str,
    before_child_name: &str,
    _message: Option<&str>,
    _actor: Option<&str>,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    if child_name == before_child_name {
        return Ok(());
    }

    let mut map = repo.nodes.write().await;
    let key = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent_path)
        .map(|(k, _)| k.clone());
    if let Some(k) = key {
        if let Some(p) = map.get_mut(&k) {
            if let Some(idx) = p.children.iter().position(|c| c == child_name) {
                p.children.remove(idx);
            }
            let pos = p
                .children
                .iter()
                .position(|c| c == before_child_name)
                .unwrap_or(p.children.len());
            p.children.insert(pos, child_name.to_string());
            // Update parent's updated_at timestamp for publish tracking
            p.updated_at = Some(chrono::Utc::now());
        }
    }
    Ok(())
}

/// Moves a child to be positioned immediately after another child.
pub(super) async fn move_child_after(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    child_name: &str,
    after_child_name: &str,
    _message: Option<&str>,
    _actor: Option<&str>,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    if child_name == after_child_name {
        return Ok(());
    }

    let mut map = repo.nodes.write().await;
    let key = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent_path)
        .map(|(k, _)| k.clone());
    if let Some(k) = key {
        if let Some(p) = map.get_mut(&k) {
            if let Some(idx) = p.children.iter().position(|c| c == child_name) {
                p.children.remove(idx);
            }
            let pos = p
                .children
                .iter()
                .position(|c| c == after_child_name)
                .map(|i| i + 1)
                .unwrap_or(p.children.len());
            p.children.insert(pos, child_name.to_string());
            // Update parent's updated_at timestamp for publish tracking
            p.updated_at = Some(chrono::Utc::now());
        }
    }
    Ok(())
}
