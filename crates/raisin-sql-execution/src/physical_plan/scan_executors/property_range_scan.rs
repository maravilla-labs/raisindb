// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Property range scan executor.
//!
//! Scans the property index for values within a bounded range.
//! Optimal for range queries like:
//! - `WHERE created_at > now()`
//! - `WHERE updated_at < '2024-01-01'`
//! - `WHERE created_at > X AND created_at < Y`

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage, StorageScope};

/// Execute a PropertyRangeScan operator.
///
/// Scans the property index for values within a bounded range.
///
/// # Performance
/// - O(k) where k is the number of matching nodes
/// - Uses RocksDB seek to jump directly to the lower bound
pub async fn execute_property_range_scan<S: Storage + 'static>(
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
        projection,
        filter,
        property_name,
        lower_bound,
        upper_bound,
        ascending,
        limit,
    ) = match plan {
        PhysicalPlan::PropertyRangeScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            projection,
            filter,
            property_name,
            lower_bound,
            upper_bound,
            ascending,
            limit,
            ..
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            projection.clone(),
            filter.clone(),
            property_name.clone(),
            lower_bound.clone(),
            upper_bound.clone(),
            *ascending,
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for property range scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();
    let qualifier = alias.clone().unwrap_or_else(|| table.clone());
    let locales = get_locales_to_use(&ctx_clone);
    let limit_count = limit.unwrap_or(usize::MAX);

    // Convert string bounds to PropertyValue bounds for the storage layer
    use raisin_models::timestamp::StorageTimestamp;

    let parse_bound =
        |bound: &Option<(String, bool)>, prop_name: &str| -> Option<(PropertyValue, bool)> {
            bound.as_ref().map(|(val, inclusive)| {
                // Check if this is a timestamp property
                if prop_name == "__created_at" || prop_name == "__updated_at" {
                    if let Ok(nanos) = val.parse::<i128>() {
                        if let Some(timestamp) = StorageTimestamp::from_nanos(nanos as i64) {
                            return (PropertyValue::Date(timestamp), *inclusive);
                        }
                    }
                }
                (PropertyValue::String(val.clone()), *inclusive)
            })
        };

    let lower_pv = parse_bound(&lower_bound, &property_name);
    let upper_pv = parse_bound(&upper_bound, &property_name);

    tracing::info!(
        "PropertyRangeScan: {} (lower={:?}, upper={:?}) {} limit={:?}",
        property_name,
        lower_bound,
        upper_bound,
        if ascending { "ASC" } else { "DESC" },
        limit
    );

    Ok(Box::pin(try_stream! {
        let entries = storage
            .property_index()
            .scan_property_range(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                &property_name,
                lower_pv.as_ref().map(|(v, i)| (v, *i)),
                upper_pv.as_ref().map(|(v, i)| (v, *i)),
                false,
                ascending,
                limit,
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        let mut emitted = 0usize;

        for entry in entries {
            if emitted >= limit_count {
                break;
            }

            let node_opt = storage
                .nodes()
                .get(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &entry.node_id,
                    ctx_clone.max_revision.as_ref(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;

            if let Some(node) = node_opt {
                if node.path == "/" {
                    continue;
                }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match rls_filter::filter_node(node, auth, &scope) {
                        Some(n) => n,
                        None => continue,
                    }
                } else {
                    node
                };

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
                            Err(e) => Err(e)?,
                        }
                    }

                    yield row;
                    emitted += 1;
                }
            }
        }
    }))
}
