//! Ordered children index key functions
//!
//! Keys for maintaining ordered child relationships using fractional indexing.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Ordered children key (revision-aware with fractional indexing)
///
/// Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~rev}\0{child_id}
pub fn ordered_child_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    order_label: &str,
    revision: &HLC,
    child_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("ordered")
        .push(parent_id)
        .push(order_label)
        .push_revision(revision)
        .push(child_id)
        .build()
}

/// Prefix for scanning all ordered children of a parent at HEAD
pub fn ordered_children_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("ordered")
        .push(parent_id)
        .build_prefix()
}

/// Prefix for finding a specific child's current order label
pub fn ordered_child_specific_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
) -> Vec<u8> {
    ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id)
}

/// Metadata key for caching the last inserted child's order label
///
/// Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0\xFF\xFFMETA\0LAST
pub fn last_child_metadata_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("ordered")
        .push(parent_id)
        .push("\u{FFFF}META")
        .push("LAST")
        .build()
}
