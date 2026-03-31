// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reference index scan executor.
//!
//! Uses the reverse reference index (ref_rev CF) to find nodes that reference
//! a specific target. Optimal for queries like:
//! `WHERE REFERENCES('workspace:/path/to/target')`

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, ReferenceIndexRepository, Storage, StorageScope};
use std::time::Instant;

/// Execute a ReferenceIndexScan operator.
///
/// Uses the reverse reference index (ref_rev CF) to find nodes that reference
/// a specific target.
///
/// # Performance
/// - O(k) where k is the number of nodes referencing the target
/// - Uses RocksDB prefix iterator on ref_rev CF
/// - Much faster than full table scan with reference property check
pub async fn execute_reference_index_scan<S: Storage + 'static>(
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
        target_workspace,
        target_path,
        projection,
        limit,
    ) = match plan {
        PhysicalPlan::ReferenceIndexScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            target_workspace,
            target_path,
            projection,
            limit,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            target_workspace.clone(),
            target_path.clone(),
            projection.clone(),
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for reference index scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let qualifier = alias.unwrap_or(table);
    let ctx_clone = ctx.clone();

    tracing::info!(
        "   ReferenceIndexScan: target='{}:{}', workspace='{}', branch='{}', limit={:?}",
        target_workspace,
        target_path,
        workspace,
        branch,
        limit
    );

    Ok(Box::pin(try_stream! {
        let referencing_nodes = storage
            .reference_index()
            .find_referencing_nodes(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                &target_workspace, &target_path, false,
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        tracing::debug!(
            "   ReferenceIndexScan: found {} referencing nodes",
            referencing_nodes.len()
        );

        let locales_to_use = get_locales_to_use(&ctx_clone);

        let mut emitted = 0usize;
        let mut seen_nodes = std::collections::HashSet::new();
        let start_time = Instant::now();

        for (source_node_id, _property_path) in referencing_nodes {
            // Skip duplicates
            if !seen_nodes.insert(source_node_id.clone()) {
                continue;
            }

            if let Some(lim) = limit {
                if emitted >= lim {
                    tracing::debug!("ReferenceIndexScan early termination: reached limit of {}", lim);
                    break;
                }
            }

            if emitted > SCAN_COUNT_CEILING {
                tracing::warn!("ReferenceIndexScan count limit reached: {} nodes", emitted);
                break;
            }

            if emitted % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                tracing::warn!(
                    "ReferenceIndexScan time limit reached: {:?} elapsed, {} nodes",
                    start_time.elapsed(), emitted
                );
                break;
            }

            let node = match storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &source_node_id, max_revision.as_ref())
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?
            {
                Some(n) => n,
                None => {
                    tracing::warn!("Node ID {} from reference index not found, skipping", source_node_id);
                    continue;
                }
            };

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

                let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                emitted += 1;
                yield row;

                if let Some(lim) = limit {
                    if emitted >= lim { break; }
                }
            }
        }

        tracing::debug!(
            "   ReferenceIndexScan completed: emitted {} rows in {:?}",
            emitted, start_time.elapsed()
        );
    }))
}
