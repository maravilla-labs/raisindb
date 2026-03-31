// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Bulk UPDATE operations and shared bulk helpers for complex WHERE clauses.
//!
//! When a WHERE clause cannot be resolved to a single node identifier,
//! these functions find all matching nodes via optimized SELECT and
//! apply the update operation in batches.

use crate::physical_plan::executor::ExecutionContext;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::TypedExpr;
use raisin_storage::Storage;

use super::helpers::{eval_expr_with_row_to_property_value, node_to_row};
use super::node_helpers::apply_assignment_to_node;

/// Batch size threshold for bulk operations.
///
/// When a bulk UPDATE/DELETE affects more than this number of rows,
/// the operation is split into batches with separate commits.
pub(super) const BULK_BATCH_SIZE: usize = 5000;

/// Extract the original commit message from the active transaction context.
pub(super) async fn get_transaction_message<S>(ctx: &ExecutionContext<S>) -> Option<String>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use raisin_storage::transactional::TransactionalContext;

    let tx_lock = ctx.transaction_context.read().await;
    if let Some(txn_ctx) = tx_lock.as_ref() {
        txn_ctx.get_message().ok().flatten()
    } else {
        None
    }
}

/// Extract the actor from the active transaction context.
pub(super) async fn get_transaction_actor<S>(ctx: &ExecutionContext<S>) -> String
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use raisin_storage::transactional::TransactionalContext;

    let tx_lock = ctx.transaction_context.read().await;
    if let Some(txn_ctx) = tx_lock.as_ref() {
        txn_ctx
            .get_actor()
            .ok()
            .flatten()
            .unwrap_or_else(|| "system".to_string())
    } else {
        "system".to_string()
    }
}

/// Execute a bulk UPDATE on all nodes matching a complex filter.
///
/// 1. Finds all matching node IDs using optimized SELECT
/// 2. If count > BULK_BATCH_SIZE, routes to batched execution
/// 3. Otherwise, updates all nodes in a single transaction
pub async fn execute_bulk_update_workspace<S>(
    workspace: &str,
    assignments: &[(String, TypedExpr)],
    filter: &TypedExpr,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use raisin_storage::transactional::TransactionalContext;

    tracing::info!(
        "Executing bulk UPDATE on workspace '{}' with complex filter",
        workspace
    );

    let matching_ids = find_matching_node_ids(workspace, filter, ctx).await?;

    tracing::info!(
        "Found {} nodes matching filter in workspace '{}'",
        matching_ids.len(),
        workspace
    );

    if matching_ids.is_empty() {
        return Ok(0);
    }

    // Large dataset: batched execution
    if matching_ids.len() > BULK_BATCH_SIZE {
        let original_message = get_transaction_message(ctx).await;
        let actor = get_transaction_actor(ctx).await;
        tracing::info!(
            "Large dataset ({} nodes) - using batched execution with {} buckets",
            matching_ids.len(),
            matching_ids.len().div_ceil(BULK_BATCH_SIZE)
        );
        return execute_bulk_update_batched(
            workspace,
            assignments,
            matching_ids,
            original_message,
            &actor,
            ctx,
        )
        .await;
    }

    // Small dataset: single-transaction flow
    let tx_lock = ctx.transaction_context.read().await;
    let has_active_tx = tx_lock.is_some();
    drop(tx_lock);

    if !has_active_tx {
        let actor = get_transaction_actor(ctx).await;
        let txn_ctx = ctx.storage.begin_context().await?;
        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(&ctx.branch)?;
        txn_ctx.set_message("SQL UPDATE")?;
        txn_ctx.set_actor(&actor)?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        let affected =
            update_nodes_in_txn(txn_ctx.as_ref(), workspace, &matching_ids, assignments).await?;

        txn_ctx.commit().await?;

        tracing::info!(
            "Bulk UPDATE (auto-commit) completed: {} rows affected",
            affected
        );

        return Ok(affected);
    }

    // Active transaction exists - use it
    let tx_lock = ctx.transaction_context.read().await;
    let txn_ctx = tx_lock.as_ref().unwrap();

    let affected =
        update_nodes_in_txn(txn_ctx.as_ref(), workspace, &matching_ids, assignments).await?;

    tracing::info!("Bulk UPDATE completed: {} rows affected", affected);

    Ok(affected)
}

/// Shared update loop for a set of nodes within a transaction context.
async fn update_nodes_in_txn(
    txn_ctx: &dyn raisin_storage::transactional::TransactionalContext,
    workspace: &str,
    node_ids: &[String],
    assignments: &[(String, TypedExpr)],
) -> Result<usize, Error> {
    let mut affected = 0;
    for node_id in node_ids {
        let mut node = txn_ctx.get_node(workspace, node_id).await?.ok_or_else(|| {
            Error::Validation(format!("Node '{}' not found during bulk update", node_id))
        })?;

        let row = node_to_row(&node, workspace);

        for (col_name, value_expr) in assignments {
            let prop_value = eval_expr_with_row_to_property_value(value_expr, &row)?;
            apply_assignment_to_node(&mut node, col_name, prop_value)?;
        }

        txn_ctx.put_node(workspace, &node).await?;
        affected += 1;

        if affected % 1000 == 0 {
            tracing::debug!("Bulk UPDATE progress: {} rows updated", affected);
        }
    }
    Ok(affected)
}

/// Execute bulk UPDATE in batches with separate commits.
async fn execute_bulk_update_batched<S>(
    workspace: &str,
    assignments: &[(String, TypedExpr)],
    matching_ids: Vec<String>,
    original_message: Option<String>,
    actor: &str,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use raisin_storage::transactional::TransactionalContext;

    let total_buckets = matching_ids.len().div_ceil(BULK_BATCH_SIZE);
    let mut total_affected = 0;

    for (bucket_idx, chunk) in matching_ids.chunks(BULK_BATCH_SIZE).enumerate() {
        let bucket_num = bucket_idx + 1;

        let txn_ctx = ctx.storage.begin_context().await?;
        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(&ctx.branch)?;

        let message = match &original_message {
            Some(msg) => format!(
                "{} [raisin:sql bucket {} of {}]",
                msg, bucket_num, total_buckets
            ),
            None => format!(
                "SQL UPDATE [raisin:sql bucket {} of {}]",
                bucket_num, total_buckets
            ),
        };
        txn_ctx.set_message(&message)?;
        txn_ctx.set_actor(actor)?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        let mut affected = 0;
        for node_id in chunk {
            if let Some(mut node) = txn_ctx.get_node(workspace, node_id).await? {
                let row = node_to_row(&node, workspace);
                for (col_name, value_expr) in assignments {
                    let prop_value = eval_expr_with_row_to_property_value(value_expr, &row)?;
                    apply_assignment_to_node(&mut node, col_name, prop_value)?;
                }
                txn_ctx.put_node(workspace, &node).await?;
                affected += 1;
            }
        }

        txn_ctx.commit().await?;
        total_affected += affected;

        tracing::info!(
            bucket = bucket_num,
            total_buckets = total_buckets,
            affected = affected,
            total_affected = total_affected,
            "Committed bulk UPDATE bucket"
        );
    }

    tracing::info!(
        "Batched bulk UPDATE completed: {} rows affected in {} buckets",
        total_affected,
        total_buckets
    );

    Ok(total_affected)
}

/// Find all node IDs matching a filter using optimized SELECT query execution.
///
/// Builds a `SELECT id FROM workspace WHERE <filter>` query and executes it
/// through the full optimization pipeline (PropertyIndexScan, FullTextScan, etc.).
pub(super) async fn find_matching_node_ids<S>(
    workspace: &str,
    filter: &TypedExpr,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<String>, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use crate::physical_plan::executor::execute_plan;
    use crate::physical_plan::planner::PhysicalPlanner;
    use futures::StreamExt;
    use raisin_sql::analyzer::{AnalyzedQuery, TableRef};
    use raisin_sql::logical_plan::PlanBuilder;
    use raisin_sql::optimizer::Optimizer;
    use raisin_sql::DataType;
    use raisin_sql::StaticCatalog;

    tracing::debug!(
        "Building SELECT query to find matching node IDs in workspace '{}'",
        workspace
    );

    let analyzed_query = AnalyzedQuery {
        ctes: vec![],
        projection: vec![(
            TypedExpr::column(workspace.to_string(), "id".to_string(), DataType::Text),
            None,
        )],
        from: vec![TableRef {
            table: workspace.to_string(),
            alias: None,
            workspace: Some(workspace.to_string()),
            table_function: None,
            subquery: None,
            lateral_function: None,
        }],
        joins: vec![],
        selection: Some(filter.clone()),
        group_by: vec![],
        aggregates: vec![],
        order_by: vec![],
        limit: None,
        offset: None,
        max_revision: ctx.max_revision,
        branch_override: None,
        locales: vec![],
        distinct: None,
    };

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(workspace.to_string());
    let plan_builder = PlanBuilder::new(&catalog);
    let logical_plan = plan_builder
        .build(&raisin_sql::analyzer::AnalyzedStatement::Query(
            analyzed_query,
        ))
        .map_err(|e| Error::Validation(format!("Failed to build logical plan: {}", e)))?;

    let optimizer = Optimizer::default();
    let optimized = optimizer.optimize(logical_plan);

    let physical_planner = PhysicalPlanner::with_context(
        ctx.tenant_id.to_string(),
        ctx.repo_id.to_string(),
        ctx.branch.to_string(),
        workspace.to_string(),
    );
    let physical_plan = physical_planner.plan(&optimized)?;

    tracing::debug!("Physical plan for ID lookup: {}", physical_plan.describe());

    let mut stream = execute_plan(&physical_plan, ctx).await?;
    let mut ids = Vec::new();

    while let Some(row_result) = stream.next().await {
        let row = row_result?;
        if let Some(PropertyValue::String(id)) = row.columns.get("id") {
            ids.push(id.clone());
        }
    }

    Ok(ids)
}
