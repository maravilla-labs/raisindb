// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Path-based order scan via DFS traversal.
//!
//! Performs ORDER BY path using streaming DFS traversal of the ORDERED_CHILDREN
//! index. Supports ascending and descending order by reversing child iteration.

use super::super::helpers::resolve_node_for_locale;
use super::super::node_to_row::node_to_row;
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute ORDER BY path via DFS traversal of ORDERED_CHILDREN.
pub(super) async fn execute_path_order_scan<S: Storage + 'static>(
    storage: std::sync::Arc<S>,
    ctx_clone: ExecutionContext<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    qualifier: String,
    locales: Vec<String>,
    projection: Option<Vec<String>>,
    filter: Option<raisin_sql::analyzer::TypedExpr>,
    ascending: bool,
    target_rows: usize,
) -> Result<RowStream, ExecutionError> {
    Ok(Box::pin(try_stream! {
        tracing::info!(
            "   PropertyOrderScan: path ordering via ORDERED_CHILDREN (streaming DFS) direction={} limit_hint={}",
            if ascending { "ASC" } else { "DESC" },
            target_rows
        );

        let mut emitted = 0usize;
        let mut stack = vec![raisin_models::nodes::ROOT_NODE_ID.to_string()];

        while let Some(current_id) = stack.pop() {
            if target_rows != usize::MAX && emitted >= target_rows {
                break;
            }

            let node_opt = storage
                .nodes()
                .get(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &current_id,
                    ctx_clone.max_revision.as_ref(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;

            let Some(node) = node_opt else {
                tracing::warn!(
                    "PropertyOrderScan (path): orphaned ordered-children entry - node '{}' not found, skipping",
                    current_id
                );
                continue;
            };

            let node = if let Some(ref auth) = ctx_clone.auth_context {
                let scope = PermissionScope::new(&workspace, &branch);
                match rls_filter::filter_node(node, auth, &scope) {
                    Some(n) => n,
                    None => continue,
                }
            } else {
                node
            };

            {
                if node.path != "/" {
                    for locale in &locales {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                            Some(n) => n,
                            None => continue,
                        };

                        let row = node_to_row(
                            &translated_node,
                            &qualifier,
                            &workspace,
                            &projection,
                            &ctx_clone,
                            locale,
                        )
                        .await?;

                        if let Some(ref filter_expr) = filter {
                            match eval_expr(filter_expr, &row) {
                                Ok(raisin_sql::analyzer::Literal::Boolean(true)) => {}
                                Ok(raisin_sql::analyzer::Literal::Boolean(false))
                                | Ok(raisin_sql::analyzer::Literal::Null) => continue,
                                Ok(other) => {
                                    Err(Error::Validation(format!(
                                        "Filter expression must return boolean, got {:?}",
                                        other
                                    )))?;
                                }
                                Err(e) => {
                                    Err(e)?;
                                }
                            }
                        }

                        emitted += 1;
                        yield row;

                        if target_rows != usize::MAX && emitted >= target_rows {
                            break;
                        }
                    }
                }

                // Stream child IDs
                let parent_lookup = if node.path == "/" {
                    "/"
                } else {
                    &node.id
                };

                let child_ids = storage
                    .nodes()
                    .stream_ordered_child_ids(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        parent_lookup,
                        ctx_clone.max_revision.as_ref(),
                    )
                    .await
                    .map_err(|e| ExecutionError::Backend(e.to_string()))?;

                if ascending {
                    for child_id in child_ids.into_iter().rev() {
                        stack.push(child_id);
                    }
                } else {
                    for child_id in child_ids {
                        stack.push(child_id);
                    }
                }
            }
        }
    }))
}
