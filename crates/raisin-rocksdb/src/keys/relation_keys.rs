//! Relation forward, reverse, and global key functions
//!
//! Keys for graph relationship indexes: workspace-scoped forward/reverse
//! and cross-workspace global relation indexes.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Relation forward key (revision-aware): source_node -> target_node
pub fn relation_forward_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    source_node_id: &str,
    relation_type: &str,
    revision: &HLC,
    target_node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("rel")
        .push(source_node_id)
        .push(relation_type)
        .push_revision(revision)
        .push(target_node_id)
        .build()
}

/// Relation reverse key (revision-aware): target_node -> source_node
pub fn relation_reverse_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    target_node_id: &str,
    relation_type: &str,
    revision: &HLC,
    source_node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("rel_rev")
        .push(target_node_id)
        .push(relation_type)
        .push_revision(revision)
        .push(source_node_id)
        .build()
}

/// Relation forward prefix: scan all outgoing relations from a node
pub fn relation_forward_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    source_node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("rel")
        .push(source_node_id)
        .build_prefix()
}

/// Relation reverse prefix: scan all incoming relations to a node
pub fn relation_reverse_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    target_node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("rel_rev")
        .push(target_node_id)
        .build_prefix()
}

/// Global relation key (revision-aware): cross-workspace relationship index
pub fn relation_global_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    relation_type: &str,
    revision: &HLC,
    source_workspace: &str,
    source_node_id: &str,
    target_workspace: &str,
    target_node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("rel_global")
        .push(relation_type)
        .push_revision(revision)
        .push(source_workspace)
        .push(source_node_id)
        .push(target_workspace)
        .push(target_node_id)
        .build()
}

/// Global relation prefix: scan all relationships across all workspaces
pub fn relation_global_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("rel_global")
        .build_prefix()
}

/// Global relation type prefix: scan all relationships of a specific type
pub fn relation_global_type_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    relation_type: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("rel_global")
        .push(relation_type)
        .build_prefix()
}
