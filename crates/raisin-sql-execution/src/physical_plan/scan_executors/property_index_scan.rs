// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Property index scan executor.
//!
//! Uses the property_index column family to find nodes by property value.
//! Optimal for queries like: `WHERE properties->>'status' = 'published'`

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage, StorageScope};
use std::time::Instant;

/// Execute a PropertyIndexScan operator.
///
/// Uses the property_index column family to find nodes by property value.
pub async fn execute_property_index_scan<S: Storage + 'static>(
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
        property_name,
        property_value,
        projection,
        limit,
    ) = match plan {
        PhysicalPlan::PropertyIndexScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            property_name,
            property_value,
            projection,
            limit,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            property_name.clone(),
            property_value.clone(),
            projection.clone(),
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for property index scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();

    tracing::info!(
        "   PropertyIndexScan: property='{}', value='{}', workspace='{}', branch='{}', limit={:?}",
        property_name,
        property_value,
        workspace,
        branch,
        limit
    );

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        let prop_value = raisin_models::nodes::properties::PropertyValue::String(property_value.clone());

        tracing::debug!("   Looking up nodes by property index with limit...");
        let node_ids = storage
            .property_index()
            .find_by_property_with_limit(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &property_name, &prop_value, false, limit)
            .await?;

        tracing::info!("   PropertyIndexScan found {} node IDs", node_ids.len());

        let mut emitted = 0;
        let mut safety_scanned = 0usize;
        let start_time = Instant::now();

        for node_id in node_ids {
            if let Some(lim) = limit {
                if emitted >= lim {
                    break;
                }
            }

            safety_scanned += 1;

            if safety_scanned > SCAN_COUNT_CEILING {
                tracing::warn!("PropertyIndexScan count limit reached: {} nodes checked", safety_scanned);
                break;
            }

            if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                tracing::warn!("PropertyIndexScan time limit reached: {:?} elapsed, {} nodes checked",
                               start_time.elapsed(), safety_scanned);
                break;
            }

            let node_opt = storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &node_id, None)
                .await?;

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

                for locale in &locales_to_use {
                    let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                    yield row;
                    emitted += 1;
                }
            }
        }
    }))
}
