// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Bulk DELETE operations for complex WHERE clauses.
//!
//! Handles deletion of nodes matching complex filters, including
//! batched execution for large datasets (>5000 nodes).

use crate::physical_plan::executor::ExecutionContext;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_sql::analyzer::TypedExpr;
use raisin_storage::Storage;

use super::bulk_operations::{
    find_matching_node_ids, get_transaction_actor, get_transaction_message, BULK_BATCH_SIZE,
};

/// Execute a bulk DELETE on all nodes matching a complex filter.
pub async fn execute_bulk_delete_workspace<S>(
    workspace: &str,
    filter: &TypedExpr,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    tracing::info!(
        "Executing bulk DELETE on workspace '{}' with complex filter",
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

    delete_nodes_by_ids(workspace, matching_ids, ctx).await
}

/// Delete a provided set of node IDs (already resolved) with batching/auto-commit behavior.
pub(super) async fn delete_nodes_by_ids<S>(
    workspace: &str,
    matching_ids: Vec<String>,
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    use raisin_storage::transactional::TransactionalContext;

    if matching_ids.is_empty() {
        return Ok(0);
    }

    // Batched path for large deletions
    if matching_ids.len() > BULK_BATCH_SIZE {
        let original_message = get_transaction_message(ctx).await;
        let actor = get_transaction_actor(ctx).await;
        return execute_bulk_delete_batched(workspace, matching_ids, original_message, &actor, ctx)
            .await;
    }

    let tx_lock = ctx.transaction_context.read().await;
    let has_active_tx = tx_lock.is_some();
    drop(tx_lock);

    if !has_active_tx {
        let actor = get_transaction_actor(ctx).await;
        let txn_ctx = ctx.storage.begin_context().await?;
        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(&ctx.branch)?;
        txn_ctx.set_message("SQL DELETE")?;
        txn_ctx.set_actor(&actor)?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        let affected = delete_nodes_in_txn(txn_ctx.as_ref(), workspace, &matching_ids).await?;

        txn_ctx.commit().await?;
        return Ok(affected);
    }

    // Use existing transaction context
    let tx_lock = ctx.transaction_context.read().await;
    let txn_ctx = tx_lock.as_ref().unwrap();

    delete_nodes_in_txn(txn_ctx.as_ref(), workspace, &matching_ids).await
}

/// Shared delete loop for a set of nodes within a transaction context.
async fn delete_nodes_in_txn(
    txn_ctx: &dyn raisin_storage::transactional::TransactionalContext,
    workspace: &str,
    node_ids: &[String],
) -> Result<usize, Error> {
    let mut affected = 0;
    for node_id in node_ids {
        txn_ctx.delete_node(workspace, node_id).await?;
        affected += 1;
        if affected % 1000 == 0 {
            tracing::debug!("Bulk DELETE progress: {} rows deleted", affected);
        }
    }
    Ok(affected)
}

/// Execute bulk DELETE in batches with separate commits.
async fn execute_bulk_delete_batched<S>(
    workspace: &str,
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
                "SQL DELETE [raisin:sql bucket {} of {}]",
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
            txn_ctx.delete_node(workspace, node_id).await?;
            affected += 1;
        }

        txn_ctx.commit().await?;
        total_affected += affected;

        tracing::info!(
            bucket = bucket_num,
            total_buckets = total_buckets,
            affected = affected,
            total_affected = total_affected,
            "Committed bulk DELETE bucket"
        );
    }

    tracing::info!(
        "Batched bulk DELETE completed: {} rows deleted in {} buckets",
        total_affected,
        total_buckets
    );

    Ok(total_affected)
}
