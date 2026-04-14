//! Graph projection key functions
//!
//! Keys for graph projection configurations that define subgraph extraction
//! rules for branches.

use super::KeyBuilder;

/// Graph projection key: {tenant}\0{repo}\0graph_projection\0{branch}\0{config_id}
pub fn graph_projection_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    config_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_projection")
        .push(branch)
        .push(config_id)
        .build()
}

/// Graph projection prefix for a branch: {tenant}\0{repo}\0graph_projection\0{branch}\0
pub fn graph_projection_branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_projection")
        .push(branch)
        .build_prefix()
}

/// Graph projection prefix for a repo: {tenant}\0{repo}\0graph_projection\0
pub fn graph_projection_repo_prefix(tenant_id: &str, repo_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_projection")
        .build_prefix()
}
