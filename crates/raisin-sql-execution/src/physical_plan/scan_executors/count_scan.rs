// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Count scan executors.
//!
//! Optimized count operations that only count keys without deserializing node data.
//! These are 10-100x faster than full scans + HashAggregate for pure COUNT(*) queries.
//!
//! # Row-level security
//!
//! The raw-key counts below are only sound for a caller row-level security
//! waves through wholesale (system callers and system admins). For anyone else
//! they would report the count of nodes the caller cannot read a single row of
//! — a workspace-wide row-count leak, and, with a property filter, an existence
//! oracle over indexed properties. So when [`auth_requires_rls`] is true the
//! executors fall back to an RLS-aware count that materializes each candidate
//! node and passes it through `rls_filter_node_graph`, exactly as every scan
//! executor does. See `helpers::auth_requires_rls`.

use super::helpers::auth_requires_rls;
use super::index_recheck::node_still_matches;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage, StorageScope};

/// Execute a CountScan operator.
///
/// Optimized count operation that only counts keys without deserializing node data.
///
/// # Performance
/// - Memory: O(1) - only stores count and deduplication set
/// - Time: O(n) - iterates all keys once
/// - For 2M nodes: ~10MB memory vs 1-4GB for full scan
pub async fn execute_count_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (tenant_id, repo_id, branch, workspace, max_revision) = match plan {
        PhysicalPlan::CountScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            max_revision,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            *max_revision,
        ),
        _ => {
            return Err(ExecutionError::Backend(
                "execute_count_scan called with non-CountScan plan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let max_rev = max_revision.or(ctx.max_revision);

    // RLS-aware path: a non-system, non-admin caller must never learn the count
    // of a workspace it cannot read. Count only the nodes that survive RLS.
    if auth_requires_rls(ctx) {
        let count =
            count_all_with_rls(ctx, &tenant_id, &repo_id, &branch, &workspace, max_rev).await?;
        return Ok(single_count_row(count));
    }

    // Execute count_all - this is fast and memory-efficient
    let count = storage
        .nodes()
        .count_all(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            max_rev.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    Ok(single_count_row(count as i64))
}

/// Execute a PropertyIndexCountScan operator.
///
/// Optimized count operation for queries with property filters.
/// Counts nodes matching a property value without deserializing node data.
///
/// # Performance
/// - Memory: O(1) - only stores count and deduplication set
/// - Time: O(n) where n is number of matching index entries
/// - For 65K matching nodes: ~10ms vs 1688ms for full scan
///
/// # Example Queries
/// ```sql
/// SELECT COUNT(*) FROM nodes WHERE node_type = 'Post'
/// SELECT COUNT(*) FROM nodes WHERE properties->>'status' = 'published'
/// ```
pub async fn execute_property_index_count_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (tenant_id, repo_id, branch, workspace, properties) = match plan {
        PhysicalPlan::PropertyIndexCountScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            properties,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            properties.clone(),
        ),
        _ => {
            return Err(ExecutionError::Backend(
                "execute_property_index_count_scan called with non-PropertyIndexCountScan plan"
                    .to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();

    // RLS-aware path: count only the matching nodes the caller may actually
    // read. Without this a filtered COUNT(*) is a per-value existence oracle
    // over any indexed property, in workspaces the caller cannot read.
    if auth_requires_rls(ctx) {
        let count =
            count_by_property_with_rls(ctx, &tenant_id, &repo_id, &branch, &workspace, &properties)
                .await?;
        return Ok(single_count_row(count));
    }

    // Sum the per-value index counts. Multiple pairs come from IN/OR expansion
    // over the same column, whose per-value row sets are disjoint.
    //
    // Only USER properties reach here: the planner never pushes a COUNT on a
    // pseudo-property down to raw index keys (see
    // `plan_dispatch::aggregate::try_plan_property_index_count`).
    let snapshot = ctx.statement_snapshot().await?;
    let mut count = 0usize;
    for (property_name, property_value) in &properties {
        let prop_value = PropertyValue::String(property_value.clone());

        // Execute count_by_property - this is fast and memory-efficient
        count += storage
            .property_index()
            .count_by_property(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                property_name,
                &prop_value,
                false, // published_only = false (count all nodes)
                Some(&snapshot),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;
    }

    Ok(single_count_row(count as i64))
}

/// A single-row `count_star` stream, the shape every count executor yields.
fn single_count_row(count: i64) -> RowStream {
    Box::pin(try_stream! {
        let mut row = Row::new();
        row.insert("count_star".to_string(), PropertyValue::Integer(count));
        yield row;
    })
}

/// Count every node in the workspace the caller may read, applying RLS.
///
/// A DFS over the ORDERED_CHILDREN index, mirroring `execute_table_scan`: a
/// node denied by RLS is not counted, but its children are still traversed
/// (they may carry different grants). The scan budget is the same one the table
/// scan enforces, so a restricted caller counting a huge workspace fails the
/// same way the equivalent `SELECT` would rather than returning a wrong count.
async fn count_all_with_rls<S: Storage + 'static>(
    ctx: &ExecutionContext<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    max_revision: Option<raisin_hlc::HLC>,
) -> Result<i64, ExecutionError> {
    let storage = ctx.storage.clone();
    let auth = ctx
        .auth_context
        .as_ref()
        .expect("auth_requires_rls implies an auth context");
    let scope = PermissionScope::new(workspace, branch);
    let start = std::time::Instant::now();

    let mut count: i64 = 0;
    let mut safety_scanned = 0usize;
    let mut stack: Vec<String> = storage
        .nodes()
        .stream_ordered_child_ids(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/",
            max_revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?
        .into_iter()
        .rev()
        .collect();

    while let Some(node_id) = stack.pop() {
        safety_scanned += 1;
        if safety_scanned > SCAN_COUNT_CEILING {
            return Err(super::scan_count_budget_exceeded(
                safety_scanned,
                start.elapsed(),
            ));
        }
        if safety_scanned % TIME_CHECK_INTERVAL == 0 && start.elapsed() > SCAN_TIME_LIMIT {
            return Err(super::scan_time_budget_exceeded(
                safety_scanned,
                start.elapsed(),
            ));
        }

        let node = match storage
            .nodes()
            .get(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &node_id,
                max_revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?
        {
            Some(n) => n,
            None => continue,
        };

        // Children are traversed regardless of whether this node is visible.
        let children = storage
            .nodes()
            .stream_ordered_child_ids(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &node.id,
                max_revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;
        for child_id in children.into_iter().rev() {
            stack.push(child_id);
        }

        // The synthetic root is never a countable row (table_scan skips it too).
        if node.path == "/" {
            continue;
        }

        let visible = super::helpers::rls_filter_node_graph(
            &*storage,
            node,
            auth,
            &scope,
            tenant_id,
            repo_id,
            branch,
            max_revision.as_ref(),
        )
        .await
        .is_some();
        if visible {
            count += 1;
        }
    }

    Ok(count)
}

/// Count the property-matching nodes the caller may read, applying RLS.
///
/// The pairs come from `IN` / same-column `OR` expansion, whose per-value row
/// sets are disjoint, so their filtered counts sum just as the raw index counts
/// do. Each candidate is materialized and passed through RLS.
async fn count_by_property_with_rls<S: Storage + 'static>(
    ctx: &ExecutionContext<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    properties: &[(String, String)],
) -> Result<i64, ExecutionError> {
    let storage = ctx.storage.clone();
    let auth = ctx
        .auth_context
        .as_ref()
        .expect("auth_requires_rls implies an auth context");
    let scope = PermissionScope::new(workspace, branch);
    // The index read and the node decodes see one revision.
    let max_revision = Some(ctx.statement_snapshot().await?);
    let start = std::time::Instant::now();

    let mut count: i64 = 0;
    let mut safety_scanned = 0usize;

    for (property_name, property_value) in properties {
        let prop_value = PropertyValue::String(property_value.clone());
        let node_ids = storage
            .property_index()
            .find_by_property(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                property_name,
                &prop_value,
                false,
                max_revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        for node_id in node_ids {
            safety_scanned += 1;
            if safety_scanned > SCAN_COUNT_CEILING {
                return Err(super::scan_count_budget_exceeded(
                    safety_scanned,
                    start.elapsed(),
                ));
            }
            if safety_scanned % TIME_CHECK_INTERVAL == 0 && start.elapsed() > SCAN_TIME_LIMIT {
                return Err(super::scan_time_budget_exceeded(
                    safety_scanned,
                    start.elapsed(),
                ));
            }

            let node = match storage
                .nodes()
                .get(
                    StorageScope::new(tenant_id, repo_id, branch, workspace),
                    &node_id,
                    max_revision.as_ref(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?
            {
                Some(n) => n,
                None => continue,
            };
            if node.path == "/" {
                continue;
            }
            // The rows are decoded here anyway, so an orphan entry costs
            // nothing to rule out — for a pseudo-property it must be.
            if !node_still_matches(&node, property_name, property_value) {
                continue;
            }

            let visible = super::helpers::rls_filter_node_graph(
                &*storage,
                node,
                auth,
                &scope,
                tenant_id,
                repo_id,
                branch,
                max_revision.as_ref(),
            )
            .await
            .is_some();
            if visible {
                count += 1;
            }
        }
    }

    Ok(count)
}
