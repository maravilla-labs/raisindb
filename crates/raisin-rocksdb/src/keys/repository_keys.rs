//! Repository-level key functions
//!
//! Keys for branches, workspaces, tags, revisions, snapshots, trees,
//! repositories, tenants, deployments, deltas, and versions.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Workspace key: {tenant}\0{repo}\0workspaces\0{workspace_id}
pub fn workspace_key(tenant_id: &str, repo_id: &str, workspace_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("workspaces")
        .push(workspace_id)
        .build()
}

/// Branch key: {tenant}\0{repo}\0branches\0{branch_name}
pub fn branch_key(tenant_id: &str, repo_id: &str, branch_name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("branches")
        .push(branch_name)
        .build()
}

/// Tag key: {tenant}\0{repo}\0tags\0{tag_name}
pub fn tag_key(tenant_id: &str, repo_id: &str, tag_name: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("tags")
        .push(tag_name)
        .build()
}

/// Revision metadata key: {tenant}\0{repo}\0revisions\0{~revision}
pub fn revision_meta_key(tenant_id: &str, repo_id: &str, revision: &HLC) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("revisions")
        .push_revision(revision)
        .build()
}

/// Node change index key: {tenant}\0{repo}\0node_changes\0{node_id}\0{~revision}
pub fn node_change_key(tenant_id: &str, repo_id: &str, node_id: &str, revision: &HLC) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("node_changes")
        .push(node_id)
        .push_revision(revision)
        .build()
}

/// NodeType change index key: {tenant}\0{repo}\0node_type_changes\0{name}\0{~revision}
pub fn node_type_change_key(
    tenant_id: &str,
    repo_id: &str,
    node_type_name: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("node_type_changes")
        .push(node_type_name)
        .push_revision(revision)
        .build()
}

/// Archetype change index key: {tenant}\0{repo}\0archetype_changes\0{name}\0{~revision}
pub fn archetype_change_key(
    tenant_id: &str,
    repo_id: &str,
    archetype_name: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("archetype_changes")
        .push(archetype_name)
        .push_revision(revision)
        .build()
}

/// ElementType change index key: {tenant}\0{repo}\0element_type_changes\0{name}\0{~revision}
pub fn element_type_change_key(
    tenant_id: &str,
    repo_id: &str,
    element_type_name: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("element_type_changes")
        .push(element_type_name)
        .push_revision(revision)
        .build()
}

/// Node snapshot key: {tenant}\0{repo}\0snapshots\0{node_id}\0{~revision}
pub fn node_snapshot_key(tenant_id: &str, repo_id: &str, node_id: &str, revision: &HLC) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("snapshots")
        .push(node_id)
        .push_revision(revision)
        .build()
}

/// Translation snapshot key: {tenant}\0{repo}\0trans_snapshots\0{node_id}\0{locale}\0{~revision}
pub fn translation_snapshot_key(
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    locale: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("trans_snapshots")
        .push(node_id)
        .push(locale)
        .push_revision(revision)
        .build()
}

/// Tree key: {tenant}\0{repo}\0trees\0{tree_id_hex}
pub fn tree_key(tenant_id: &str, repo_id: &str, tree_id: &[u8; 32]) -> Vec<u8> {
    let tree_id_hex = hex::encode(tree_id);
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("trees")
        .push(&tree_id_hex)
        .build()
}

/// Repository info key: {tenant}\0repos\0{repo_id}
pub fn repository_key(tenant_id: &str, repo_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("repos")
        .push(repo_id)
        .build()
}

/// Tenant registry key: tenants\0{tenant_id}
pub fn tenant_key(tenant_id: &str) -> Vec<u8> {
    KeyBuilder::new().push("tenants").push(tenant_id).build()
}

/// Deployment registry key: deployments\0{tenant_id}\0{deployment_key}
pub fn deployment_key(tenant_id: &str, deployment_key: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push("deployments")
        .push(tenant_id)
        .push(deployment_key)
        .build()
}

/// Workspace delta key: {tenant}\0{repo}\0{branch}\0{workspace}\0delta\0{operation}\0{path}
pub fn workspace_delta_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    operation: &str,
    path: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("delta")
        .push(operation)
        .push(path)
        .build()
}

/// Version key: {tenant}\0{repo}\0versions\0{node_id}\0{version}
pub fn version_key(tenant_id: &str, repo_id: &str, node_id: &str, version: i32) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("versions")
        .push(node_id)
        .push(&version.to_string())
        .build()
}
