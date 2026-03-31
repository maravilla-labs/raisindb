//! List operations for in-memory node repository
//!
//! These functions handle list operations that need has_children computation.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::ListOptions;

use super::basic_ops;
use super::InMemoryNodeRepo;

/// List nodes by type with optional has_children computation
pub(crate) async fn list_by_type(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
    options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    let mut nodes =
        basic_ops::list_by_type(repo, tenant_id, repo_id, branch, workspace, node_type).await?;
    if options.compute_has_children {
        compute_has_children(
            repo, tenant_id, repo_id, branch, workspace, &mut nodes, &options,
        )
        .await?;
    }
    Ok(nodes)
}

/// List nodes by parent with optional has_children computation
pub(crate) async fn list_by_parent(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent: &str,
    options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    let mut nodes =
        basic_ops::list_by_parent(repo, tenant_id, repo_id, branch, workspace, parent).await?;
    if options.compute_has_children {
        compute_has_children(
            repo, tenant_id, repo_id, branch, workspace, &mut nodes, &options,
        )
        .await?;
    }
    Ok(nodes)
}

/// List all nodes with optional has_children computation
pub(crate) async fn list_all(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    let mut nodes = basic_ops::list_all(repo, tenant_id, repo_id, branch, workspace).await?;
    if options.compute_has_children {
        compute_has_children(
            repo, tenant_id, repo_id, branch, workspace, &mut nodes, &options,
        )
        .await?;
    }
    Ok(nodes)
}

/// List root nodes with optional has_children computation
pub(crate) async fn list_root(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    let mut nodes = super::tree_ops::list_root(repo, tenant_id, repo_id, branch, workspace).await?;
    if options.compute_has_children {
        compute_has_children(
            repo, tenant_id, repo_id, branch, workspace, &mut nodes, &options,
        )
        .await?;
    }
    Ok(nodes)
}

/// List children with optional has_children computation
pub(crate) async fn list_children(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    let mut nodes =
        super::tree_ops::list_children(repo, tenant_id, repo_id, branch, workspace, parent_path)
            .await?;
    if options.compute_has_children {
        compute_has_children(
            repo, tenant_id, repo_id, branch, workspace, &mut nodes, &options,
        )
        .await?;
    }
    Ok(nodes)
}

/// Compute has_children for a set of nodes
async fn compute_has_children(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    nodes: &mut [models::nodes::Node],
    options: &ListOptions,
) -> Result<()> {
    for node in nodes.iter_mut() {
        node.has_children = Some(
            basic_ops::has_children(repo, tenant_id, repo_id, branch, workspace, &node.id).await?,
        );
    }
    // TODO: use options.max_revision for point-in-time queries
    // (currently in-memory storage only tracks latest state)
    let _ = options;
    Ok(())
}
