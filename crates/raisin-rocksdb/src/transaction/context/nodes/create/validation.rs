//! Validation logic for node creation
//!
//! This module handles validation of node create and update operations.

use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{CreateNodeOptions, UpdateNodeOptions};

use crate::transaction::RocksDBTransaction;

/// Validate a node for creation
///
/// Validates that the node can be created by checking:
/// - Parent allows child (if enabled)
/// - Workspace allows type
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node to validate
///
/// # Errors
///
/// Returns error if validation fails
pub(super) async fn validate_create(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    validate_parent: bool,
) -> Result<()> {
    let options = CreateNodeOptions {
        validate_schema: false, // Transaction doesn't validate schema (caller's responsibility)
        validate_parent_allows_child: validate_parent,
        validate_workspace_allows_type: true,
        operation_meta: None,
    };

    tx.node_repo
        .validate_for_create(tenant_id, repo_id, branch, workspace, node, &options)
        .await
}

/// Validate a node for update
///
/// Validates that the node can be updated.
/// Allows type changes for migrations.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node to validate
///
/// # Errors
///
/// Returns error if validation fails
#[allow(dead_code)]
pub(super) async fn validate_update(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
) -> Result<()> {
    let options = UpdateNodeOptions {
        validate_schema: false, // Transaction doesn't validate schema (caller's responsibility)
        allow_type_change: true, // Transactions allow type changes (used in migrations)
        operation_meta: None,
    };

    tx.node_repo
        .validate_for_update(tenant_id, repo_id, branch, workspace, node, &options)
        .await
}

/// Validate a node for update using an already-fetched existing node
///
/// This is the transaction-aware version that doesn't re-fetch from the database.
/// Use this when you already have the existing node from the transaction's read cache
/// (read-your-writes semantics).
///
/// # Arguments
///
/// * `existing_node` - The existing node fetched via transaction's read cache
/// * `new_node` - The updated node to validate
///
/// # Errors
///
/// Returns error if type change is not allowed
pub(super) fn validate_update_with_existing(existing_node: &Node, new_node: &Node) -> Result<()> {
    // Transactions allow type changes (used in migrations)
    // If we wanted to block type changes, we'd check here:
    // if existing_node.node_type != new_node.node_type {
    //     return Err(Error::Validation(...));
    // }

    // Currently we allow all updates since the node was already verified to exist
    // via the transaction's read-your-writes cache
    Ok(())
}

/// Check unique property constraints before writing a node
///
/// This function validates that all properties marked as `unique: true` in the NodeType
/// do not conflict with existing nodes in the workspace.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node to validate
///
/// # Returns
///
/// * `Ok(())` - All unique constraints are satisfied
/// * `Err(Error::Validation)` - A unique constraint violation was detected
pub(super) async fn check_unique_constraints(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
) -> Result<()> {
    tx.node_repo
        .check_unique_constraints(node, tenant_id, repo_id, branch, workspace)
        .await
}

/// Detect changes in parent and path
///
/// Returns information about what changed in an update:
/// - Whether parent changed
/// - Whether path changed
/// - Old parent value (if changed)
/// - Old path value (if exists)
///
/// # Arguments
///
/// * `existing_node` - The existing node (if any)
/// * `new_node` - The new node
///
/// # Returns
///
/// Tuple of (parent_changed, path_changed, old_parent, old_path)
pub(super) fn detect_changes(
    existing_node: Option<&Node>,
    new_node: &Node,
) -> (bool, bool, Option<String>, Option<String>) {
    let parent_changed = existing_node
        .as_ref()
        .map(|old| old.parent != new_node.parent)
        .unwrap_or(false);

    let path_changed = existing_node
        .as_ref()
        .map(|old| old.path != new_node.path)
        .unwrap_or(false);

    let old_parent = existing_node.as_ref().and_then(|old| old.parent.clone());

    let old_path = existing_node.as_ref().map(|old| old.path.clone());

    (parent_changed, path_changed, old_parent, old_path)
}
