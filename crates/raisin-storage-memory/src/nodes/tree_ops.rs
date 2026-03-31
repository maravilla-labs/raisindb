use raisin_error::Result;
use raisin_models as models;

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Lists all root nodes (nodes without a parent) in the workspace.
pub(super) async fn list_root(
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
        })
        .map(|(_, n)| n.clone())
        .collect();
    // stable order by name to make pagination/tests deterministic
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Lists all direct children of a parent node, maintaining the parent's child order.
pub(super) async fn list_children(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;
    // Filter by comparing derived parent PATH with the passed-in parent PATH
    let mut out: Vec<_> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix) && n.parent_path().as_deref() == Some(parent_path)
        })
        .map(|(_, n)| n.clone())
        .collect();
    if let Some((_, parent_node)) = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent_path)
    {
        let order = &parent_node.children;
        out.sort_by(|a, b| {
            let ia = order.iter().position(|n| n == &a.name);
            let ib = order.iter().position(|n| n == &b.name);
            match (ia, ib) {
                (Some(ia), Some(ib)) => ia.cmp(&ib),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            }
        });
    } else {
        out.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(out)
}

/// Deletes a node by path and all its descendants.
///
/// Also removes the node from its parent's children list.
pub(super) async fn delete_by_path(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<bool> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let mut map = repo.nodes.write().await;

    // locate node by path
    let target_opt = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == path)
        .map(|(k, n)| (k.clone(), n.clone()));
    let Some((target_key, target_node)) = target_opt else {
        return Ok(false);
    };

    // remove target
    let removed = map.remove(&target_key).is_some();

    // detach from parent children vec if parent exists
    if removed {
        // remove descendants as well (paths starting with path + "/")
        let keys_to_remove: Vec<String> = map
            .iter()
            .filter(|(k, n)| {
                k.starts_with(&workspace_prefix)
                    && (n.path == path || n.path.starts_with(&(path.to_string() + "/")))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys_to_remove {
            map.remove(&k);
        }

        // update parent's children vector - derive parent PATH from target_node.path
        if let Some(parent_path) = target_node.parent_path() {
            // find parent by path
            if let Some((parent_key, _)) = map
                .iter()
                .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent_path)
                .map(|(k, n)| (k.clone(), n.clone()))
            {
                if let Some(parent_node) = map.get_mut(&parent_key) {
                    parent_node.children.retain(|c| c != &target_node.name);
                }
            }
        }
    }

    Ok(removed)
}

/// Moves a node to a new path, updating all descendant paths accordingly.
///
/// Returns an error if the destination path already exists or if attempting
/// to move a node into its own descendant.
pub(super) async fn move_node(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    id: &str,
    new_path: &str,
    _operation_meta: Option<raisin_models::operations::OperationMeta>,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let node_key = NodeKey::new(tenant_id, repo_id, branch, workspace, id).to_path();
    let mut map = repo.nodes.write().await;

    // Check if destination already exists
    if let Some((existing_key, _)) = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == new_path)
        .map(|(k, n)| (k.clone(), n.clone()))
    {
        if existing_key != node_key {
            return Err(raisin_error::Error::Backend(
                "destination path already exists".into(),
            ));
        }
    }

    let Some(mut node) = map.get(&node_key).cloned() else {
        return Ok(());
    };

    let old_path = node.path.clone();
    if old_path == new_path {
        return Ok(());
    }
    if new_path.starts_with(&(old_path.clone() + "/")) {
        return Err(raisin_error::Error::Validation(
            "cannot move a node into its own descendant".into(),
        ));
    }
    node.path = new_path.to_string();

    // Update parent NAME (not path!) - extract parent name from new path
    node.parent = models::nodes::Node::extract_parent_name_from_path(new_path);

    map.insert(node_key.clone(), node);

    // update descendants paths
    let updates: Vec<(String, models::nodes::Node)> = map
        .iter()
        .filter(|(k, n)| {
            k.starts_with(&workspace_prefix) && n.path.starts_with(&(old_path.clone() + "/"))
        })
        .map(|(k, n)| {
            let mut clone = n.clone();
            clone.path = clone.path.replacen(&old_path, new_path, 1);
            (k.clone(), clone)
        })
        .collect();
    for (k, v) in updates {
        map.insert(k, v);
    }
    Ok(())
}

/// Renames a node, updating its path and all descendant paths.
///
/// Also updates the parent's children list to reflect the new name.
pub(super) async fn rename_node(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_path: &str,
    new_name: &str,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    if new_name.is_empty() || new_name.contains('/') {
        return Err(raisin_error::Error::Validation("invalid name".into()));
    }
    // compute new_path
    let new_path = if let Some(idx) = old_path.rfind('/') {
        format!("{}/{}", &old_path[..idx], new_name)
    } else {
        format!("/{}", new_name)
    };

    // find node by path
    let mut map = repo.nodes.write().await;
    if map
        .iter()
        .any(|(k, n)| k.starts_with(&workspace_prefix) && n.path == new_path)
    {
        return Err(raisin_error::Error::Backend(
            "destination path already exists".into(),
        ));
    }

    let maybe_key = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == old_path)
        .map(|(k, _)| k.clone());

    if let Some(key) = maybe_key {
        if let Some(mut node) = map.get(&key).cloned() {
            node.name = new_name.to_string();
            node.path = new_path.clone();

            // update children array on parent: replace old name with new name, or append if missing
            if let Some(parent_path) = new_path.rfind('/').map(|i| new_path[..i].to_string()) {
                if let Some((parent_key, _)) = map
                    .iter()
                    .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == parent_path)
                    .map(|(k, n)| (k.clone(), n.clone()))
                {
                    if let Some(parent_node) = map.get_mut(&parent_key) {
                        let old_name = old_path.split('/').next_back().unwrap_or("").to_string();
                        let mut replaced = false;
                        for c in &mut parent_node.children {
                            if c == &old_name {
                                *c = node.name.clone();
                                replaced = true;
                            }
                        }
                        if !replaced && !parent_node.children.iter().any(|c| c == &node.name) {
                            parent_node.children.push(node.name.clone());
                        }
                    }
                }
            }
            map.insert(key.clone(), node);

            // update descendants
            let updates: Vec<(String, models::nodes::Node)> = map
                .iter()
                .filter(|(k, n)| {
                    k.starts_with(&workspace_prefix)
                        && n.path.starts_with(&(old_path.to_string() + "/"))
                })
                .map(|(k, n)| {
                    let mut clone = n.clone();
                    clone.path = clone.path.replacen(old_path, &new_path, 1);
                    (k.clone(), clone)
                })
                .collect();
            for (k, v) in updates {
                map.insert(k, v);
            }
        }
    }
    Ok(())
}
