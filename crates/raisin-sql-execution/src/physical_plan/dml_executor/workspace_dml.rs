// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Workspace table DML operations (INSERT, UPDATE, DELETE).
//!
//! Handles operations on workspace tables (content nodes)
//! with transaction context management (active vs auto-commit).

use crate::physical_plan::executor::ExecutionContext;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::TypedExpr;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::bulk_delete::execute_bulk_delete_workspace;
use super::bulk_operations::execute_bulk_update_workspace;
use super::helpers::{
    eval_expr_to_property_value, eval_expr_with_row_to_property_value, node_to_row,
};
use super::initial_structure::create_initial_structure_children;
use super::node_helpers::{
    apply_assignment_to_node, build_node_from_columns, extract_node_identifier_from_filter,
    NodeIdentifier,
};

/// Execute INSERT on workspace table.
pub(super) async fn execute_insert_workspace<S>(
    workspace: &str,
    columns: &[String],
    values: &[Vec<TypedExpr>],
    is_upsert: bool,
    ctx: &ExecutionContext<S>,
) -> Result<(), Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage,
{
    use raisin_storage::transactional::TransactionalContext;

    let use_active_txn = {
        let tx_lock = ctx.transaction_context.read().await;
        tx_lock.is_some()
    };

    let op_name = if is_upsert { "UPSERT" } else { "INSERT" };

    if use_active_txn {
        tracing::debug!("INSERT/UPSERT using active transaction context");
        let tx_lock = ctx.transaction_context.read().await;
        let txn_ctx = tx_lock.as_ref().ok_or_else(|| {
            Error::InvalidState("Transaction context lost during execution".to_string())
        })?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        insert_rows_via_txn(txn_ctx.as_ref(), ctx, workspace, columns, values, is_upsert).await?;
    } else {
        tracing::debug!("{} using auto-commit mode", op_name);
        let txn_ctx = ctx.storage.begin_context().await?;

        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(&ctx.branch)?;
        let message = if is_upsert {
            "SQL UPSERT"
        } else {
            "SQL INSERT"
        };
        txn_ctx.set_message(message)?;
        let actor = if is_upsert {
            "sql-upsert"
        } else {
            "sql-insert"
        };
        txn_ctx.set_actor(actor)?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        insert_rows_via_txn(txn_ctx.as_ref(), ctx, workspace, columns, values, is_upsert).await?;

        txn_ctx.commit().await?;
    }

    Ok(())
}

/// Shared logic for inserting rows via a transaction context.
async fn insert_rows_via_txn<S: Storage>(
    txn_ctx: &dyn raisin_storage::transactional::TransactionalContext,
    ctx: &ExecutionContext<S>,
    workspace: &str,
    columns: &[String],
    values: &[Vec<TypedExpr>],
    is_upsert: bool,
) -> Result<(), Error> {
    for row_values in values {
        let mut col_map = IndexMap::new();
        for (col_name, value_expr) in columns.iter().zip(row_values.iter()) {
            let prop_value = eval_expr_to_property_value(value_expr)?;
            col_map.insert(col_name.clone(), prop_value);
        }

        let actor = ctx.auth_context.as_ref().map(|a| a.actor_id());
        let node = build_node_from_columns(&col_map, workspace, actor.as_deref())?;

        if is_upsert {
            txn_ctx.upsert_node(workspace, &node).await?;
        } else {
            txn_ctx.add_node(workspace, &node).await?;

            create_initial_structure_children(
                txn_ctx,
                ctx.storage.as_ref(),
                &ctx.tenant_id,
                &ctx.repo_id,
                &ctx.branch,
                workspace,
                &node,
            )
            .await?;
        }
    }

    Ok(())
}

/// Execute UPDATE on workspace table.
pub(super) async fn execute_update_workspace<S>(
    workspace: &str,
    assignments: &[(String, TypedExpr)],
    filter: &Option<TypedExpr>,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use raisin_storage::transactional::TransactionalContext;

    // Try to extract simple node identifier from WHERE clause (path or id)
    // If that fails, fall back to bulk update mode
    let node_identifier = match extract_node_identifier_from_filter(filter) {
        Ok(ident) => ident,
        Err(_) => {
            let filter_expr = filter
                .as_ref()
                .ok_or_else(|| Error::Validation("UPDATE requires a WHERE clause".to_string()))?;
            return execute_bulk_update_workspace(workspace, assignments, filter_expr, ctx).await;
        }
    };

    let use_active_txn = {
        let tx_lock = ctx.transaction_context.read().await;
        tx_lock.is_some()
    };

    if use_active_txn {
        tracing::debug!("UPDATE using active transaction context");
        let tx_lock = ctx.transaction_context.read().await;
        let txn_ctx = tx_lock.as_ref().ok_or_else(|| {
            Error::InvalidState("Transaction context lost during execution".to_string())
        })?;

        update_single_node(txn_ctx.as_ref(), workspace, &node_identifier, assignments).await?;
    } else {
        tracing::debug!("UPDATE using auto-commit mode");
        let txn_ctx = ctx.storage.begin_context().await?;
        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(&ctx.branch)?;
        txn_ctx.set_message("SQL UPDATE")?;
        txn_ctx.set_actor("sql-update")?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        update_single_node(txn_ctx.as_ref(), workspace, &node_identifier, assignments).await?;

        txn_ctx.commit().await?;
    }

    Ok(1)
}

/// Shared logic for updating a single node via transaction context.
async fn update_single_node(
    txn_ctx: &dyn raisin_storage::transactional::TransactionalContext,
    workspace: &str,
    node_identifier: &NodeIdentifier,
    assignments: &[(String, TypedExpr)],
) -> Result<(), Error> {
    let mut node = match node_identifier {
        NodeIdentifier::Id(id) => txn_ctx
            .get_node(workspace, id)
            .await?
            .ok_or_else(|| Error::Validation(format!("Node with id '{}' not found", id)))?,
        NodeIdentifier::Path(path) => txn_ctx
            .get_node_by_path(workspace, path)
            .await?
            .ok_or_else(|| Error::Validation(format!("Node at path '{}' not found", path)))?,
    };

    let row = node_to_row(&node, workspace);

    for (col_name, value_expr) in assignments {
        let prop_value = eval_expr_with_row_to_property_value(value_expr, &row)?;
        apply_assignment_to_node(&mut node, col_name, prop_value)?;
    }

    txn_ctx.put_node(workspace, &node).await?;
    Ok(())
}

/// Execute DELETE on workspace table.
///
/// Both path-based and id-based deletes cascade to delete all descendants.
/// For path-based deletes (WHERE path = '/x'), this automatically cascades by
/// collecting descendants via path-indexed scan.
///
/// For id-based deletes (WHERE id = 'xxx'), the node's path is looked up first,
/// then the same cascade logic is applied.
pub(super) async fn execute_delete_workspace<S>(
    workspace: &str,
    filter: &Option<TypedExpr>,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    // Try to extract simple node identifier from WHERE clause (path or id)
    // If that fails, fall back to bulk delete mode
    let node_identifier = match extract_node_identifier_from_filter(filter) {
        Ok(ident) => ident,
        Err(_) => {
            let filter_expr = filter
                .as_ref()
                .ok_or_else(|| Error::Validation("DELETE requires a WHERE clause".to_string()))?;
            return execute_bulk_delete_workspace(workspace, filter_expr, ctx).await;
        }
    };

    // Gather node IDs to delete (target + descendants)
    let ids_to_delete = collect_cascade_ids(&node_identifier, workspace, ctx).await?;

    super::bulk_delete::delete_nodes_by_ids(workspace, ids_to_delete, ctx).await
}

/// Collect node IDs for cascade delete (target node + all descendants).
async fn collect_cascade_ids<S>(
    node_identifier: &NodeIdentifier,
    workspace: &str,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<String>, Error>
where
    S: Storage + 'static,
{
    let mut ids_to_delete = Vec::new();

    match node_identifier {
        NodeIdentifier::Path(path) => {
            if let Some(node) = ctx
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, workspace),
                    path,
                    ctx.max_revision.as_ref(),
                )
                .await
                .map_err(|e| Error::storage(e.to_string()))?
            {
                ids_to_delete.push(node.id.clone());
                let descendants = ctx
                    .storage
                    .nodes()
                    .deep_children_flat(
                        StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, workspace),
                        path,
                        100,
                        ctx.max_revision.as_ref(),
                    )
                    .await
                    .map_err(|e| Error::storage(e.to_string()))?;
                for desc_node in descendants {
                    ids_to_delete.push(desc_node.id);
                }
            }
        }
        NodeIdentifier::Id(node_id) => {
            if let Some(node) = ctx
                .storage
                .nodes()
                .get(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, workspace),
                    node_id,
                    ctx.max_revision.as_ref(),
                )
                .await
                .map_err(|e| Error::storage(e.to_string()))?
            {
                ids_to_delete.push(node.id.clone());
                let descendants = ctx
                    .storage
                    .nodes()
                    .deep_children_flat(
                        StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, workspace),
                        &node.path,
                        100,
                        ctx.max_revision.as_ref(),
                    )
                    .await
                    .map_err(|e| Error::storage(e.to_string()))?;
                for desc_node in descendants {
                    ids_to_delete.push(desc_node.id);
                }
            }
        }
    }

    Ok(ids_to_delete)
}
