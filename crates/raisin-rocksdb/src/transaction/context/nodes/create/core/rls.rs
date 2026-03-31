//! Row-Level Security (RLS) permission checks for node operations
//!
//! Shared RLS validation logic used by both `put_node` and `add_node`.

use raisin_error::Result;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;

/// Check CREATE permission via RLS
///
/// Returns Ok(()) if the operation is allowed, or an error if denied.
/// Uses deny-by-default when no auth context is set.
pub fn check_create_permission(
    tx: &RocksDBTransaction,
    node: &Node,
    workspace: &str,
) -> Result<()> {
    let meta = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    match &meta.auth_context {
        Some(auth) => {
            use raisin_core::services::rls_filter;
            use raisin_models::permissions::PermissionScope;

            let branch_str = meta.branch.as_ref().map(|s| s.as_str()).unwrap_or("main");
            let scope = PermissionScope::new(workspace, branch_str);

            if !rls_filter::can_create_at_path(&node.path, &node.node_type, auth, &scope) {
                return Err(raisin_error::Error::PermissionDenied(format!(
                    "Cannot create {} at path '{}'",
                    node.node_type, node.path
                )));
            }
            Ok(())
        }
        None => {
            tracing::warn!(
                path = %node.path,
                "Transaction has no auth context - denying node create operation"
            );
            Err(raisin_error::Error::PermissionDenied(
                "Transaction requires auth context for node operations".to_string(),
            ))
        }
    }
}

/// Check CREATE or UPDATE permission via RLS
///
/// For new nodes: checks create permission at the path.
/// For existing nodes: checks update permission on the existing node.
/// Returns Ok(()) if the operation is allowed, or an error if denied.
pub fn check_put_permission(
    tx: &RocksDBTransaction,
    node: &Node,
    existing_node: Option<&Node>,
    workspace: &str,
) -> Result<()> {
    let meta = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    match &meta.auth_context {
        Some(auth) => {
            use raisin_core::services::rls_filter;
            use raisin_models::permissions::{Operation, PermissionScope};

            let branch_str = meta.branch.as_ref().map(|s| s.as_str()).unwrap_or("main");
            let scope = PermissionScope::new(workspace, branch_str);

            if existing_node.is_none() {
                // CREATE operation
                if !rls_filter::can_create_at_path(&node.path, &node.node_type, auth, &scope) {
                    return Err(raisin_error::Error::PermissionDenied(format!(
                        "Cannot create {} at path '{}'",
                        node.node_type, node.path
                    )));
                }
            } else if let Some(existing) = existing_node {
                // UPDATE operation
                if !rls_filter::can_perform(existing, Operation::Update, auth, &scope) {
                    return Err(raisin_error::Error::PermissionDenied(format!(
                        "Cannot update node at path '{}'",
                        existing.path
                    )));
                }
            }
            Ok(())
        }
        None => {
            tracing::warn!(
                path = %node.path,
                "Transaction has no auth context - denying put_node operation"
            );
            Err(raisin_error::Error::PermissionDenied(
                "Transaction requires auth context for node operations".to_string(),
            ))
        }
    }
}
