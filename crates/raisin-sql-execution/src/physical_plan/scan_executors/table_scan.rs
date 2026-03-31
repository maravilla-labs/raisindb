// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Table scan executor.
//!
//! Performs a full table scan using DFS traversal via the ORDERED_CHILDREN index.
//! This is the fallback scan method when no better access path is available.

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute a TableScan operator.
///
/// Performs a full table scan, optionally filtered by node_type.
/// Uses DFS traversal via the ORDERED_CHILDREN index for natural node ordering.
pub async fn execute_table_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (tenant_id, repo_id, branch, workspace, table, alias, filter, projection, limit) =
        match plan {
            PhysicalPlan::TableScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                table,
                alias,
                filter,
                projection,
                limit,
                ..
            } => (
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
                workspace.clone(),
                table.clone(),
                alias.clone(),
                filter.clone(),
                projection.clone(),
                *limit,
            ),
            _ => return Err(Error::Validation("Invalid plan for table scan".to_string())),
        };

    // Intercept pg_catalog virtual tables
    if crate::physical_plan::pg_catalog_executor::is_pg_catalog_table(&table) {
        let simple_name =
            crate::physical_plan::pg_catalog_executor::get_simple_pg_catalog_table_name(&table);
        tracing::info!("   PgCatalogScan: table='{}'", simple_name);
        return crate::physical_plan::pg_catalog_executor::execute_pg_catalog_scan(
            simple_name,
            ctx.storage.clone(),
            &tenant_id,
            &repo_id,
        )
        .await;
    }

    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let qualifier = alias.clone().unwrap_or_else(|| table.clone());
    let ctx_clone = ctx.clone();

    tracing::debug!(
        table = %table,
        filter = filter.is_some(),
        "table_scan started"
    );

    tracing::info!(
        "   TableScan: workspace='{}', branch='{}', max_revision={:?}",
        workspace,
        branch,
        max_revision
    );

    Ok(Box::pin(try_stream! {
        let locales_to_use = get_locales_to_use(&ctx_clone);

        tracing::debug!("   Starting DFS traversal via ORDERED_CHILDREN index...");
        let start = std::time::Instant::now();

        let mut emitted = 0usize;
        let mut safety_scanned = 0usize;
        let mut stack: Vec<String> = Vec::new();

        // Start DFS with root children (parent_id = "/")
        let root_children = storage
            .nodes()
            .stream_ordered_child_ids(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                "/",
                max_revision.as_ref(),
            )
            .await?;

        // Push root children onto stack in reverse order (so we pop in correct order)
        for child_id in root_children.into_iter().rev() {
            stack.push(child_id);
        }

        tracing::debug!("   Starting DFS with {} root children", stack.len());

        // DFS traversal - process one node at a time
        while let Some(node_id) = stack.pop() {
            // Early termination BEFORE fetching node (critical for performance)
            if let Some(lim) = limit {
                if emitted >= lim {
                    tracing::debug!(
                        "TableScan early termination: reached limit of {} in {}us",
                        lim,
                        start.elapsed().as_micros()
                    );
                    break;
                }
            }

            safety_scanned += 1;

            if safety_scanned > SCAN_COUNT_CEILING {
                tracing::warn!("TableScan count limit reached: {} nodes checked", safety_scanned);
                break;
            }

            if safety_scanned % TIME_CHECK_INTERVAL == 0 && start.elapsed() > SCAN_TIME_LIMIT {
                tracing::warn!("TableScan time limit reached: {:?} elapsed, {} nodes checked",
                               start.elapsed(), safety_scanned);
                break;
            }

            // Materialize node on-demand (LAST POSSIBLE MOMENT)
            let node = match storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &node_id, max_revision.as_ref())
                .await?
            {
                Some(n) => n,
                None => {
                    tracing::warn!("Node ID {} not found during DFS, skipping", node_id);
                    continue;
                }
            };

            // Apply RLS filtering if auth context is set
            let node = if let Some(ref auth) = ctx_clone.auth_context {
                let scope = PermissionScope::new(&workspace, &branch);
                match rls_filter::filter_node(node, auth, &scope) {
                    Some(n) => n,
                    None => {
                        // User doesn't have permission to see this node, skip it
                        // But still need to traverse children (they might have different permissions)
                        let children = storage
                            .nodes()
                            .stream_ordered_child_ids(
                                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                                &node_id,
                                max_revision.as_ref(),
                            )
                            .await?;

                        for child_id in children.into_iter().rev() {
                            stack.push(child_id);
                        }
                        continue;
                    }
                }
            } else {
                node
            };

            // Skip root nodes (system detail that users shouldn't see)
            if node.path == "/" {
                let children = storage
                    .nodes()
                    .stream_ordered_child_ids(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        &node_id,
                        max_revision.as_ref(),
                    )
                    .await?;

                for child_id in children.into_iter().rev() {
                    stack.push(child_id);
                }
                continue;
            }

            // Generate one row per locale (with translation if configured)
            for locale in &locales_to_use {
                let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                    Some(n) => n,
                    None => continue,
                };

                let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;

                // Apply pushed-down filter if present
                if let Some(ref filter_expr) = filter {
                    match eval_expr(filter_expr, &row) {
                        Ok(raisin_sql::analyzer::Literal::Boolean(true)) => {
                            yield row;
                            emitted += 1;
                            if let Some(lim) = limit {
                                if emitted >= lim {
                                    tracing::debug!("TableScan early termination: reached limit of {}", lim);
                                    break;
                                }
                            }
                        }
                        Ok(raisin_sql::analyzer::Literal::Boolean(false))
                        | Ok(raisin_sql::analyzer::Literal::Null) => {
                            continue;
                        }
                        Ok(other) => {
                            Err(Error::Validation(format!(
                                "Filter expression must return boolean, got {:?}",
                                other
                            )))?;
                            unreachable!();
                        }
                        Err(e) => {
                            Err(e)?;
                            unreachable!();
                        }
                    }
                } else {
                    yield row;
                    emitted += 1;
                    if let Some(lim) = limit {
                        if emitted >= lim {
                            tracing::debug!("TableScan early termination: reached limit of {}", lim);
                            break;
                        }
                    }
                }
            }

            // Continue DFS: push children of current node onto stack
            let children = storage
                .nodes()
                .stream_ordered_child_ids(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &node.id,
                    max_revision.as_ref(),
                )
                .await?;

            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }

        tracing::debug!(
            "   DFS traversal completed in {}us, emitted {} rows",
            start.elapsed().as_micros(),
            emitted
        );
    }))
}
