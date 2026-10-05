// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reference index scan executor.
//!
//! Uses the reverse reference index (ref_rev CF) to find nodes that reference
//! a specific target. Optimal for queries like:
//! `WHERE REFERENCES('workspace:/path/to/target')`

use super::batch_fetch::{chunk_len, fetch_nodes, per_locale};
use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row_owned;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{
    BranchScope, NodeRepository, ReferenceIndexRepository, Storage, StorageScope,
};
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

    tracing::debug!(
        "   ReferenceIndexScan: target='{}:{}', workspace='{}', branch='{}', limit={:?}",
        target_workspace,
        target_path,
        workspace,
        branch,
        limit
    );

    Ok(Box::pin(try_stream! {
        // The reverse reference index is keyed by the target's STABLE node id
        // (so it survives target moves), but queries name the target by path.
        // Resolve path -> id once here; an unresolvable target has no referrers.
        // NOTE: the target lives in `target_workspace` (which may differ from the
        // FROM/source `workspace` being scanned), so resolve in the TARGET's
        // workspace — otherwise cross-workspace references resolve to nothing.
        let target_id = storage
            .nodes()
            .get_node_id_by_path(
                StorageScope::new(&tenant_id, &repo_id, &branch, &target_workspace),
                &target_path,
                max_revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        let referencing_nodes = match target_id {
            Some(target_id) => storage
                .reference_index()
                .find_referencing_nodes_at(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &target_workspace, &target_id, false, max_revision.as_ref(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?,
            None => {
                tracing::debug!(
                    "   ReferenceIndexScan: target '{}:{}' not found; 0 referrers",
                    target_workspace, target_path
                );
                Vec::new()
            }
        };

        tracing::debug!(
            "   ReferenceIndexScan: found {} referencing nodes",
            referencing_nodes.len()
        );

        let locales_to_use = get_locales_to_use(&ctx_clone);

        let mut emitted = 0usize;
        let mut seen_nodes = std::collections::HashSet::new();
        let start_time = Instant::now();

        // The statement's revision and storage view for every chunk's read.
        let scan_revision = ctx_clone.statement_snapshot().await?;
        // Distinct referrers, in index order (one node can reference the
        // target from several properties).
        let ids: Vec<String> = referencing_nodes
            .into_iter()
            .map(|(source_node_id, _property_path)| source_node_id)
            .filter(|id| seen_nodes.insert(id.clone()))
            .collect();
        let mut cursor = 0;
        let mut previous = 0;

        'chunks: while cursor < ids.len() {
            let chunk = &ids[cursor..(cursor + chunk_len(limit, emitted, previous)).min(ids.len())];
            cursor += chunk.len();
            previous = chunk.len();
            let nodes = fetch_nodes(
                &ctx_clone,
                BranchScope::new(&tenant_id, &repo_id, &branch),
                &workspace,
                chunk,
                &scan_revision,
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

            for (source_node_id, node) in chunk.iter().zip(nodes) {
                if let Some(lim) = limit {
                    if emitted >= lim {
                        tracing::debug!("ReferenceIndexScan early termination: reached limit of {}", lim);
                        break 'chunks;
                    }
                }

                if emitted > SCAN_COUNT_CEILING {
                    tracing::warn!("ReferenceIndexScan count limit reached: {} nodes", emitted);
                    Err(super::scan_count_budget_exceeded(emitted, start_time.elapsed()))?;
                }

                if emitted % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                    tracing::warn!(
                        "ReferenceIndexScan time limit reached: {:?} elapsed, {} nodes",
                        start_time.elapsed(), emitted
                    );
                    Err(super::scan_time_budget_exceeded(emitted, start_time.elapsed()))?;
                }

                let Some(node) = node else {
                    tracing::warn!("Node ID {} from reference index not found, skipping", source_node_id);
                    continue;
                };

                if node.path == "/" { continue; }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, max_revision.as_ref()).await {
                        Some(n) => n,
                        None => continue,
                    }
                } else {
                    node
                };

                for (locale, node) in per_locale(node, &locales_to_use) {
                    let translated_node = match resolve_node_for_locale(node, &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    let row = node_to_row_owned(translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, None,).await?;
                    emitted += 1;
                    yield row;

                    if let Some(lim) = limit {
                        if emitted >= lim { break; }
                    }
                }
            }
        }

        tracing::debug!(
            "   ReferenceIndexScan completed: emitted {} rows in {:?}",
            emitted, start_time.elapsed()
        );
    }))
}
