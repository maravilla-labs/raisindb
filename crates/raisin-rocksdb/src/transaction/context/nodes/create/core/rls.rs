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
pub async fn check_create_permission(
    tx: &RocksDBTransaction,
    node: &Node,
    workspace: &str,
) -> Result<()> {
    // Clone auth + scope out of the metadata guard so it is released before the
    // async graph-resolver evaluation.
    let (auth_opt, tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.auth_context.clone(),
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone(),
        )
    };

    match auth_opt {
        Some(auth) => {
            use raisin_core::services::rls_filter;
            use raisin_models::permissions::PermissionScope;
            use raisin_storage::{scope::BranchScope, Storage};

            let (tid, rid): (&str, &str) = (&tenant_id, &repo_id);
            let br = branch.as_ref().map(|s| s.as_str()).unwrap_or("main");
            let scope = PermissionScope::new(workspace, br);
            // Fast path: build the graph resolver only when a `RELATES`
            // condition is present; otherwise evaluate synchronously.
            let allowed = if auth.uses_graph_rls() {
                // Write-time RLS evaluates relations at latest committed state.
                let revision = raisin_hlc::HLC::now();
                let resolver = tx
                    .storage
                    .graph_resolver(BranchScope::new(tid, rid, br), &revision);
                rls_filter::can_create_at_path_async(
                    &node.path,
                    &node.node_type,
                    &auth,
                    &scope,
                    resolver.as_deref(),
                )
                .await
            } else {
                rls_filter::can_create_at_path(&node.path, &node.node_type, &auth, &scope)
            };

            if !allowed {
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
pub async fn check_put_permission(
    tx: &RocksDBTransaction,
    node: &Node,
    existing_node: Option<&Node>,
    workspace: &str,
) -> Result<()> {
    // Clone auth + scope out of the metadata guard so it is released before the
    // async graph-resolver evaluation.
    let (auth_opt, tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.auth_context.clone(),
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone(),
        )
    };

    match auth_opt {
        Some(auth) => {
            use raisin_core::services::rls_filter;
            use raisin_models::permissions::{Operation, PermissionScope};
            use raisin_storage::{scope::BranchScope, Storage};

            let (tid, rid): (&str, &str) = (&tenant_id, &repo_id);
            let br = branch.as_ref().map(|s| s.as_str()).unwrap_or("main");
            let scope = PermissionScope::new(workspace, br);
            // Fast path: build the graph resolver only when a `RELATES`
            // condition is present; otherwise evaluate synchronously.
            let use_graph = auth.uses_graph_rls();
            // Write-time RLS evaluates relations at latest committed state.
            let revision = raisin_hlc::HLC::now();
            let resolver = if use_graph {
                tx.storage
                    .graph_resolver(BranchScope::new(tid, rid, br), &revision)
            } else {
                None
            };

            if existing_node.is_none() {
                // CREATE operation
                let allowed = if use_graph {
                    rls_filter::can_create_at_path_async(
                        &node.path,
                        &node.node_type,
                        &auth,
                        &scope,
                        resolver.as_deref(),
                    )
                    .await
                } else {
                    rls_filter::can_create_at_path(&node.path, &node.node_type, &auth, &scope)
                };
                if !allowed {
                    return Err(raisin_error::Error::PermissionDenied(format!(
                        "Cannot create {} at path '{}'",
                        node.node_type, node.path
                    )));
                }
            } else if let Some(existing) = existing_node {
                // UPDATE operation
                let allowed = if use_graph {
                    rls_filter::can_perform_async(
                        existing,
                        Operation::Update,
                        &auth,
                        &scope,
                        resolver.as_deref(),
                    )
                    .await
                } else {
                    rls_filter::can_perform(existing, Operation::Update, &auth, &scope)
                };
                if !allowed {
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
