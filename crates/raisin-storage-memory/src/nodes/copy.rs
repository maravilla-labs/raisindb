use raisin_error::Result;
use raisin_models as models;

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Copies a single node (without descendants) to a new parent location.
///
/// The copied node receives a new unique ID but retains all other properties.
/// Returns an error if the destination path already exists or if attempting
/// to copy into a descendant of itself.
///
/// # Arguments
/// * `new_name` - Optional new name for the copied node. If None, uses source name
pub(super) async fn copy_node(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    source_path: &str,
    target_parent: &str,
    new_name: Option<&str>,
    _operation_meta: Option<raisin_models::operations::OperationMeta>,
) -> Result<models::nodes::Node> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let mut map = repo.nodes.write().await;
    let src_pair = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == source_path)
        .map(|(k, n)| (k.clone(), n.clone()))
        .ok_or(raisin_error::Error::NotFound("node".into()))?;

    let mut node = src_pair.1.clone();
    if !target_parent.is_empty()
        && (target_parent == source_path
            || target_parent.starts_with(&(source_path.to_string() + "/")))
    {
        return Err(raisin_error::Error::Validation(
            "cannot copy a node into its own descendant".into(),
        ));
    }

    node.id = nanoid::nanoid!();
    // Clear publish state - copies are always unpublished
    node.published_at = None;
    node.published_by = None;

    // Use new_name if provided, otherwise keep the original name
    let name = new_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| node.name.clone());
    node.name = name.clone(); // Update the node's name

    let new_path = if target_parent.is_empty() || target_parent == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", target_parent.trim_end_matches('/'), name)
    };

    if map
        .iter()
        .any(|(k, n)| k.starts_with(&workspace_prefix) && n.path == new_path)
    {
        return Err(raisin_error::Error::Backend(
            "destination path already exists".into(),
        ));
    }

    node.path = new_path.clone();
    // Extract parent NAME from new_path (not target parent PATH!)
    node.parent = models::nodes::Node::extract_parent_name_from_path(&new_path);

    let key = NodeKey::new(tenant_id, repo_id, branch, workspace, &node.id).to_path();
    map.insert(key, node.clone());
    Ok(node)
}

/// Copies a node and all its descendants to a new parent location.
///
/// All copied nodes receive new unique IDs while maintaining their tree structure.
/// Returns the root of the copied tree.
///
/// # Arguments
/// * `new_name` - Optional new name for the root copied node. If None, uses source name
pub(super) async fn copy_node_tree(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    source_path: &str,
    target_parent: &str,
    new_name: Option<&str>,
    _operation_meta: Option<raisin_models::operations::OperationMeta>,
) -> Result<models::nodes::Node> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    // collision check for dest root
    let map_ro = repo.nodes.read().await;
    let root_src = map_ro
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == source_path)
        .map(|(_, n)| n.clone())
        .ok_or(raisin_error::Error::NotFound("node".into()))?;

    if !target_parent.is_empty()
        && (target_parent == source_path
            || target_parent.starts_with(&(source_path.to_string() + "/")))
    {
        return Err(raisin_error::Error::Validation(
            "cannot copy a node into its own descendant".into(),
        ));
    }

    // Use new_name if provided, otherwise keep the original name
    let root_name = new_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| root_src.name.clone());

    let dest_root_path = if target_parent.is_empty() || target_parent == "/" {
        format!("/{}", root_name)
    } else {
        format!("{}/{}", target_parent.trim_end_matches('/'), root_name)
    };

    if map_ro
        .iter()
        .any(|(k, n)| k.starts_with(&workspace_prefix) && n.path == dest_root_path)
    {
        return Err(raisin_error::Error::Backend(
            "destination path already exists".into(),
        ));
    }
    drop(map_ro);

    // shallow copy root with new_name
    let root = copy_node(
        repo,
        tenant_id,
        repo_id,
        branch,
        workspace,
        source_path,
        target_parent,
        new_name,
        None, // Internal recursive call - operation metadata tracked at top level
    )
    .await?;

    // copy descendants
    let to_copy: Vec<String> = {
        let map = repo.nodes.read().await;
        map.iter()
            .filter(|(k, n)| {
                k.starts_with(&workspace_prefix)
                    && n.path.starts_with(&(source_path.to_string() + "/"))
            })
            .map(|(_, n)| n.path.clone())
            .collect()
    };

    for p in to_copy {
        let rel = p
            .strip_prefix(&(source_path.to_string() + "/"))
            .unwrap_or("");
        let dest_parent = if rel.contains('/') {
            let parent_rel = rel.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
            if parent_rel.is_empty() {
                root.path.clone()
            } else {
                format!("{}/{}", root.path, parent_rel)
            }
        } else {
            root.path.clone()
        };
        let _ = copy_node(
            repo,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &p,
            &dest_parent,
            None,
            None, // Internal recursive call - operation metadata tracked at top level
        )
        .await?;
    }

    Ok(root)
}
