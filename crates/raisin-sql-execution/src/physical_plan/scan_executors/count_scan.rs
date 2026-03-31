// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Count scan executors.
//!
//! Optimized count operations that only count keys without deserializing node data.
//! These are 10-100x faster than full scans + HashAggregate for pure COUNT(*) queries.

use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_models::nodes::properties::PropertyValue;
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

    // Execute count_all - this is fast and memory-efficient
    let count = storage
        .nodes()
        .count_all(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            max_rev.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    // Return a single row with the count
    let stream = try_stream! {
        let mut row = Row::new();
        row.insert("count_star".to_string(), PropertyValue::Integer(count as i64));
        yield row;
    };

    Ok(Box::pin(stream))
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
    let (tenant_id, repo_id, branch, workspace, property_name, property_value) = match plan {
        PhysicalPlan::PropertyIndexCountScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            property_value,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            property_name.clone(),
            property_value.clone(),
        ),
        _ => {
            return Err(ExecutionError::Backend(
                "execute_property_index_count_scan called with non-PropertyIndexCountScan plan"
                    .to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();

    // Parse the property value into PropertyValue
    let prop_value = PropertyValue::String(property_value.clone());

    // Execute count_by_property - this is fast and memory-efficient
    let count = storage
        .property_index()
        .count_by_property(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            &property_name,
            &prop_value,
            false, // published_only = false (count all nodes)
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    // Return a single row with the count
    let stream = try_stream! {
        let mut row = Row::new();
        row.insert("count_star".to_string(), PropertyValue::Integer(count as i64));
        yield row;
    };

    Ok(Box::pin(stream))
}
