// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Compound index scan executor.
//!
//! Scans a compound (multi-column) index for efficient ORDER BY + filter queries.
//! Allows queries like:
//! `WHERE node_type = 'Article' AND category = 'news' ORDER BY created_at DESC LIMIT 10`
//! to execute in O(LIMIT) time by using a prefix scan.

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{
    CompoundColumnValue, CompoundIndexRepository, NodeRepository, Storage, StorageScope,
};
use std::time::Instant;

/// Execute a compound index scan.
///
/// Scans a compound (multi-column) index for efficient ORDER BY + filter queries.
/// The compound index pre-sorts data by equality columns followed by a sort column.
pub async fn execute_compound_index_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (
        tenant_id,
        repo_id,
        branch,
        workspace,
        table,
        alias,
        index_name,
        equality_columns,
        _pre_sorted,
        ascending,
        projection,
        filter,
        limit,
    ) = match plan {
        PhysicalPlan::CompoundIndexScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            index_name,
            equality_columns,
            pre_sorted,
            ascending,
            projection,
            filter,
            limit,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            index_name.clone(),
            equality_columns.clone(),
            *pre_sorted,
            *ascending,
            projection.clone(),
            filter.clone(),
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for compound index scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();

    tracing::info!(
        "   CompoundIndexScan: index='{}', equality_cols={:?}, ascending={}, workspace='{}', branch='{}', limit={:?}",
        index_name, equality_columns, ascending, workspace, branch, limit
    );

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        let compound_values: Vec<CompoundColumnValue> = equality_columns
            .iter()
            .map(|(_prop_name, value)| CompoundColumnValue::String(value.clone()))
            .collect();

        tracing::debug!(
            "   Scanning compound index '{}' with {} equality columns...",
            index_name, compound_values.len()
        );

        // Request 10x limit to account for post-index filtering
        let scan_limit = limit.map(|l| l.saturating_mul(10).max(100));
        let scan_results = storage
            .compound_index()
            .scan_compound_index(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                &index_name, &compound_values, false, ascending, scan_limit,
            )
            .await?;

        tracing::info!(
            "   CompoundIndexScan returned {} node IDs from index",
            scan_results.len()
        );

        let mut emitted = 0;
        let mut safety_scanned = 0usize;
        let start_time = Instant::now();

        for scan_entry in scan_results {
            if let Some(lim) = limit {
                if emitted >= lim { break; }
            }

            safety_scanned += 1;

            if safety_scanned > SCAN_COUNT_CEILING {
                tracing::warn!("CompoundIndexScan count limit reached: {} nodes checked", safety_scanned);
                break;
            }

            if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                tracing::warn!("CompoundIndexScan time limit reached: {:?} elapsed, {} nodes checked",
                               start_time.elapsed(), safety_scanned);
                break;
            }

            let node_opt = storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &scan_entry.node_id, ctx_clone.max_revision.as_ref())
                .await?;

            if let Some(node) = node_opt {
                if node.path == "/" { continue; }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match rls_filter::filter_node(node, auth, &scope) {
                        Some(n) => n,
                        None => continue,
                    }
                } else {
                    node
                };

                for locale in &locales_to_use {
                    let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    // Apply remaining filter if present
                    if let Some(ref filter_expr) = filter {
                        let row = node_to_row(&translated_node, &qualifier, &workspace, &None, &ctx_clone, locale).await?;
                        match eval_expr(filter_expr, &row) {
                            Ok(raisin_sql::analyzer::Literal::Boolean(true)) => {
                                // Filter passed
                            }
                            Ok(raisin_sql::analyzer::Literal::Boolean(false)) => continue,
                            Ok(_) => continue,
                            Err(e) => {
                                tracing::warn!("Filter evaluation error: {:?}", e);
                                continue;
                            }
                        }
                    }

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;

                    yield row;
                    emitted += 1;

                    if let Some(lim) = limit {
                        if emitted >= lim { break; }
                    }
                }

                if let Some(lim) = limit {
                    if emitted >= lim { break; }
                }
            }
        }

        tracing::info!(
            "   CompoundIndexScan completed: {} rows emitted",
            emitted
        );
    }))
}
