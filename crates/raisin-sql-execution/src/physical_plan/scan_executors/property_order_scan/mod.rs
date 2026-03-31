// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Property order scan executor.
//!
//! Executes ordered property scans (e.g., `ORDER BY created_at LIMIT N`).
//! Supports two modes:
//! - **Path ordering** via DFS traversal of ORDERED_CHILDREN
//! - **Property ordering** via property index scan with safety valve fallback
//!
//! # Module Structure
//!
//! - `path_order` - DFS-based ORDER BY path traversal
//! - `index_order` - Property index scan with safety valve fallback

mod index_order;
mod path_order;

use super::helpers::get_locales_to_use;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use raisin_error::Error;
use raisin_storage::Storage;

/// Execute an ordered property scan (e.g., ORDER BY created_at LIMIT N).
pub async fn execute_property_order_scan<S: Storage + 'static>(
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
        ascending,
        limit_hint,
    ) = match plan {
        PhysicalPlan::PropertyOrderScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            projection,
            filter,
            property_name,
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
            *ascending,
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for property order scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();
    let qualifier = alias.clone().unwrap_or_else(|| table.clone());
    let locales = get_locales_to_use(&ctx_clone);
    let target_rows = limit_hint;

    // Check if this is an ORDER BY path query - use hierarchical traversal
    if property_name == "__path" {
        return path_order::execute_path_order_scan(
            storage,
            ctx_clone,
            tenant_id,
            repo_id,
            branch,
            workspace,
            qualifier,
            locales,
            projection,
            filter,
            ascending,
            target_rows,
        )
        .await;
    }

    // Use property index scan for system properties (created_at, updated_at) and custom properties
    if property_name.starts_with("__") {
        tracing::debug!(
            "PropertyOrderScan: using system property index for '{}'",
            property_name
        );
    } else {
        tracing::debug!(
            "PropertyOrderScan: using custom property index for '{}' (has_filter={})",
            property_name,
            filter.is_some()
        );
        if filter.is_none() {
            tracing::info!(
                "PropertyOrderScan: custom property '{}' without filter — may scan many index entries",
                property_name
            );
        }
    }

    index_order::execute_property_index_order_scan(
        storage,
        ctx_clone,
        tenant_id,
        repo_id,
        branch,
        workspace,
        qualifier,
        locales,
        projection,
        filter,
        property_name,
        ascending,
        target_rows,
    )
    .await
}
