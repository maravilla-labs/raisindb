// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! ORDER/REORDER execution for reordering sibling nodes.
//!
//! Reorders a node relative to a sibling by resolving both node
//! references, validating they share the same parent, then calling
//! the appropriate reorder method (before/after).

use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute a physical ORDER operation.
///
/// Reorders a node relative to a sibling by:
/// 1. Resolving source and target node references (path or ID) to paths
/// 2. Validating that both nodes are siblings (same parent)
/// 3. Calling the appropriate repository method (move_child_before or move_child_after)
/// 4. Returning a single row with affected_rows = 1
pub async fn execute_order<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    source: &'a raisin_sql::ast::order::NodeReference,
    target: &'a raisin_sql::ast::order::NodeReference,
    position: &'a raisin_sql::ast::order::OrderPosition,
    workspace: &'a Option<String>,
    branch_override: &'a Option<String>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    use raisin_sql::ast::order::OrderPosition;

    let workspace_id = workspace
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or(&ctx.workspace);

    let branch = branch_override
        .as_ref()
        .map(|b| b.as_str())
        .unwrap_or(&ctx.branch);

    // Step 1: Resolve node references to paths
    let source_path = resolve_node_reference(source, workspace_id, branch, ctx).await?;
    let target_path = resolve_node_reference(target, workspace_id, branch, ctx).await?;

    tracing::debug!(
        "ORDER: Resolved source={} target={} position={:?}",
        source_path,
        target_path,
        position
    );

    // Step 2: Extract parent path and child names, validate same parent
    let (source_parent, source_name) = split_path_to_parent_and_name(&source_path)?;
    let (target_parent, target_name) = split_path_to_parent_and_name(&target_path)?;

    if source_parent != target_parent {
        return Err(Error::Validation(format!(
            "ORDER: Nodes must be siblings. Source '{}' has parent '{}', target '{}' has parent '{}'",
            source_path, source_parent, target_path, target_parent
        )));
    }

    if source_path == target_path {
        return Err(Error::Validation(
            "ORDER: Cannot order a node relative to itself".to_string(),
        ));
    }

    // Step 3: Execute reorder via transaction context
    let use_active_txn = {
        let tx_lock = ctx.transaction_context.read().await;
        tx_lock.is_some()
    };

    if use_active_txn {
        use raisin_storage::transactional::TransactionalContext;
        tracing::debug!("ORDER using active transaction context");
        let tx_lock = ctx.transaction_context.read().await;
        let txn_ctx = tx_lock.as_ref().ok_or_else(|| {
            Error::InvalidState("Transaction context lost during execution".to_string())
        })?;

        reorder_in_txn(
            txn_ctx.as_ref(),
            workspace_id,
            &source_parent,
            &source_name,
            &target_name,
            position,
        )
        .await?;
    } else {
        tracing::debug!("ORDER using auto-commit mode");
        let txn_ctx = ctx.storage.begin_context().await?;

        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(branch)?;
        let position_str = match position {
            OrderPosition::Above => "BEFORE",
            OrderPosition::Below => "AFTER",
        };
        txn_ctx.set_message(&format!(
            "ORDER {} {} {}",
            source_path, position_str, target_path
        ))?;
        txn_ctx.set_actor("sql-order")?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        reorder_in_txn(
            txn_ctx.as_ref(),
            workspace_id,
            &source_parent,
            &source_name,
            &target_name,
            position,
        )
        .await?;

        txn_ctx.commit().await?;
    }

    tracing::info!(
        "ORDER: Moved '{}' {} '{}' in workspace '{}'",
        source_path,
        match position {
            OrderPosition::Above => "ABOVE",
            OrderPosition::Below => "BELOW",
        },
        target_path,
        workspace_id
    );

    let mut result_row = Row::new();
    result_row.insert("affected_rows".to_string(), PropertyValue::Integer(1));

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// Perform the reorder operation within a transaction context.
async fn reorder_in_txn(
    txn_ctx: &dyn raisin_storage::transactional::TransactionalContext,
    workspace_id: &str,
    parent_path: &str,
    source_name: &str,
    target_name: &str,
    position: &raisin_sql::ast::order::OrderPosition,
) -> Result<(), Error> {
    use raisin_sql::ast::order::OrderPosition;

    match position {
        OrderPosition::Above => {
            txn_ctx
                .reorder_child_before(workspace_id, parent_path, source_name, target_name)
                .await?;
        }
        OrderPosition::Below => {
            txn_ctx
                .reorder_child_after(workspace_id, parent_path, source_name, target_name)
                .await?;
        }
    }
    Ok(())
}

/// Resolve a NodeReference (path or ID) to an actual path.
pub(super) async fn resolve_node_reference<S: Storage + 'static>(
    node_ref: &raisin_sql::ast::order::NodeReference,
    workspace: &str,
    branch: &str,
    ctx: &ExecutionContext<S>,
) -> Result<String, Error> {
    use raisin_sql::ast::order::NodeReference;

    match node_ref {
        NodeReference::Path(path) => {
            let node = ctx
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace),
                    path,
                    None,
                )
                .await?
                .ok_or_else(|| Error::NotFound(format!("Node at path '{}' not found", path)))?;
            Ok(node.path)
        }
        NodeReference::Id(id) => {
            let node = ctx
                .storage
                .nodes()
                .get(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace),
                    id,
                    None,
                )
                .await?
                .ok_or_else(|| Error::NotFound(format!("Node with ID '{}' not found", id)))?;
            Ok(node.path)
        }
    }
}

/// Split a path into parent path and child name.
///
/// Example: "/content/pages/home" -> ("/content/pages", "home")
pub(super) fn split_path_to_parent_and_name(path: &str) -> Result<(String, String), Error> {
    let path = path.trim_end_matches('/');

    match path.rsplit_once('/') {
        Some(("", name)) => Ok(("/".to_string(), name.to_string())),
        Some((parent, name)) => Ok((parent.to_string(), name.to_string())),
        None => Err(Error::Validation(format!(
            "Invalid path '{}': cannot extract parent and name",
            path
        ))),
    }
}
