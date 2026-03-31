//! Reference forward and reverse index key functions
//!
//! Keys for tracking outgoing and incoming references between nodes.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Reference index key (forward): {tenant}\0{repo}\0{branch}\0{workspace}\0ref{_pub}\0{node_id}\0{property_path}
#[deprecated(
    since = "0.1.0",
    note = "Use reference_forward_key_versioned for revision-aware storage"
)]
pub fn reference_forward_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    property_path: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "ref_pub" } else { "ref" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(node_id)
        .push(property_path)
        .build()
}

/// Revision-aware reference index key (forward)
pub fn reference_forward_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    property_path: &str,
    revision: &HLC,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "ref_pub" } else { "ref" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(node_id)
        .push(property_path)
        .push_revision(revision)
        .build()
}

/// Reference index key (reverse): {tenant}\0{repo}\0{branch}\0{workspace}\0ref_rev{_pub}\0{target_workspace}\0{target_path}\0{source_node_id}\0{property_path}
#[deprecated(
    since = "0.1.0",
    note = "Use reference_reverse_key_versioned for revision-aware storage"
)]
pub fn reference_reverse_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    target_workspace: &str,
    target_path: &str,
    source_node_id: &str,
    property_path: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "ref_rev_pub" } else { "ref_rev" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(target_workspace)
        .push(target_path)
        .push(source_node_id)
        .push(property_path)
        .build()
}

/// Revision-aware reference index key (reverse)
pub fn reference_reverse_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    target_workspace: &str,
    target_path: &str,
    source_node_id: &str,
    property_path: &str,
    revision: &HLC,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "ref_rev_pub" } else { "ref_rev" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(target_workspace)
        .push(target_path)
        .push(source_node_id)
        .push(property_path)
        .push_revision(revision)
        .build()
}

/// Reference reverse prefix: scan all incoming references to a target node
pub fn reference_reverse_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    target_workspace: &str,
    target_path: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "ref_rev_pub" } else { "ref_rev" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(target_workspace)
        .push(target_path)
        .build_prefix()
}
