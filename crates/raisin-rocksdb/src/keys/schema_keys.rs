//! Schema key functions: NodeType, Archetype, ElementType
//!
//! Keys for storing and retrieving schema definitions with versioning support.

use super::KeyBuilder;
use raisin_hlc::HLC;

// --- NodeType Keys ---

/// NodeType key: {tenant}\0{repo}\0{branch}\0nodetypes\0{name}\0{~revision}
pub fn nodetype_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("nodetypes")
        .push(name)
        .push_revision(revision)
        .build()
}

/// Legacy NodeType key without branch/revision (for backward compatibility)
pub fn nodetype_key(tenant_id: &str, repo_id: &str, name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("nodetypes")
        .push(name)
        .build()
}

/// NodeType name prefix: scan all revisions of a specific NodeType
pub fn nodetype_name_prefix(tenant_id: &str, repo_id: &str, branch: &str, name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("nodetypes")
        .push(name)
        .build_prefix()
}

/// NodeType branch prefix: scan all NodeTypes within a branch
pub fn nodetype_branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("nodetypes")
        .build_prefix()
}

/// NodeType version index key
pub fn nodetype_version_index_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
    version: i32,
) -> Vec<u8> {
    let version_str = version.to_string();
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("nodetype_versions")
        .push(name)
        .push(&version_str)
        .build()
}

// --- Archetype Keys ---

pub fn archetype_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("archetypes")
        .push(name)
        .push_revision(revision)
        .build()
}

pub fn archetype_key(tenant_id: &str, repo_id: &str, name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("archetypes")
        .push(name)
        .build()
}

pub fn archetype_name_prefix(tenant_id: &str, repo_id: &str, branch: &str, name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("archetypes")
        .push(name)
        .build_prefix()
}

pub fn archetype_branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("archetypes")
        .build_prefix()
}

pub fn archetype_version_index_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
    version: i32,
) -> Vec<u8> {
    let version_str = version.to_string();
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("archetype_versions")
        .push(name)
        .push(&version_str)
        .build()
}

// --- ElementType Keys ---

pub fn element_type_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("element_types")
        .push(name)
        .push_revision(revision)
        .build()
}

pub fn element_type_key(tenant_id: &str, repo_id: &str, name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("element_types")
        .push(name)
        .build()
}

pub fn element_type_name_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("element_types")
        .push(name)
        .build_prefix()
}

pub fn element_type_branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("element_types")
        .build_prefix()
}

pub fn element_type_version_index_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    name: &str,
    version: i32,
) -> Vec<u8> {
    let version_str = version.to_string();
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("element_type_versions")
        .push(name)
        .push(&version_str)
        .build()
}
