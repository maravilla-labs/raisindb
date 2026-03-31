//! Graph cache key functions and admin user keys
//!
//! Keys for precomputed graph algorithm results (PageRank, Louvain, etc.)
//! and system-level admin user keys.

use super::KeyBuilder;

/// Graph cache key: {tenant}\0{repo}\0graph_cache\0{branch}\0{config_id}\0{node_id}
pub fn graph_cache_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    config_id: &str,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_cache")
        .push(branch)
        .push(config_id)
        .push(node_id)
        .build()
}

/// Graph cache prefix for a config: {tenant}\0{repo}\0graph_cache\0{branch}\0{config_id}\0
pub fn graph_cache_config_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    config_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_cache")
        .push(branch)
        .push(config_id)
        .build_prefix()
}

/// Graph cache prefix for a branch: {tenant}\0{repo}\0graph_cache\0{branch}\0
pub fn graph_cache_branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_cache")
        .push(branch)
        .build_prefix()
}

/// Graph cache metadata key: {tenant}\0{repo}\0graph_cache\0{branch}\0{config_id}\0_meta
pub fn graph_cache_meta_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    config_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("graph_cache")
        .push(branch)
        .push(config_id)
        .push("_meta")
        .build()
}

/// Admin user key: sys\0{tenant}\0users\0{username}
pub fn admin_user_key(tenant_id: &str, username: &str) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(b"sys");
    key.push(0);
    key.extend_from_slice(tenant_id.as_bytes());
    key.push(0);
    key.extend_from_slice(b"users");
    key.push(0);
    key.extend_from_slice(username.as_bytes());
    key
}
