// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Property index scan executor.
//!
//! Uses the property_index column family to find nodes by property value.
//! Optimal for queries like: `WHERE properties->>'status' = 'published'`

use super::batch_fetch::{chunk_len, fetch_nodes, per_locale};
use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::index_recheck::{is_pseudo_property, json_member_equals, node_still_matches};
use super::node_to_row::node_to_row_owned;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{BranchScope, PropertyIndexRepository, Storage, StorageScope};
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
        verifies_value,
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
            verifies_value,
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
            *verifies_value,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for property index scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();

    tracing::debug!(
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

        // Timestamp pseudo-properties are planned as decimal microseconds and
        // looked up as an Integer, which the storage reader encodes exactly as
        // the timestamp writer keys them. Everything else is its stored text.
        let prop_value = match (property_name.as_str(), property_value.parse::<i64>()) {
            ("__created_at" | "__updated_at", Ok(micros)) => {
                raisin_models::nodes::properties::PropertyValue::Integer(micros)
            }
            _ => raisin_models::nodes::properties::PropertyValue::String(property_value.clone()),
        };

        // ONE revision for the whole scan, shared by the index read and every
        // node decode — the statement's snapshot. Two separate HEAD reads (one
        // in the index, one here) could straddle a commit and pair an index
        // answer from one revision with node records from another.
        let scan_revision = ctx_clone.statement_snapshot().await?;

        // A pseudo-property is answered by the index alone (no JSON residual
        // above this scan), so its candidates are re-checked against the decoded
        // node — see `index_recheck`.
        let pseudo = is_pseudo_property(&property_name);

        let mut emitted = 0;
        let mut safety_scanned = 0usize;
        let start_time = Instant::now();

        // The index is first asked for `limit` ids. A candidate can still be
        // dropped (missing at the snapshot, denied by RLS, an orphan entry), and
        // a dropped candidate must not cost the caller a row: if the first pass
        // came back full but delivered too few rows, a second, unlimited pass
        // continues past the ids already seen.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut index_limit = limit;
        loop {
            tracing::debug!("   Looking up nodes by property index with limit {:?}...", index_limit);
            let node_ids = storage
                .property_index()
                .find_by_property_with_limit(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &property_name,
                    &prop_value,
                    false,
                    Some(&scan_revision),
                    index_limit,
                )
                .await?;
            tracing::debug!("   PropertyIndexScan found {} node IDs", node_ids.len());
            let truncated = index_limit.is_some_and(|l| node_ids.len() >= l);

            // Ids not seen in an earlier pass, read in chunks (see `batch_fetch`).
            let fresh: Vec<String> = node_ids
                .into_iter()
                .filter(|id| seen.insert(id.clone()))
                .collect();
            let mut cursor = 0;
            let mut previous = 0;
            while cursor < fresh.len() {
                if limit.is_some_and(|lim| emitted >= lim) {
                    break;
                }
                let chunk = &fresh[cursor..(cursor + chunk_len(limit, emitted, previous)).min(fresh.len())];
                cursor += chunk.len();
                previous = chunk.len();

                for _ in chunk {
                    safety_scanned += 1;
                    if safety_scanned > SCAN_COUNT_CEILING {
                        tracing::warn!("PropertyIndexScan count limit reached: {} nodes checked", safety_scanned);
                        Err(super::scan_count_budget_exceeded(safety_scanned, start_time.elapsed()))?;
                    }
                    if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                        tracing::warn!("PropertyIndexScan time limit reached: {:?} elapsed, {} nodes checked",
                                       start_time.elapsed(), safety_scanned);
                        Err(super::scan_time_budget_exceeded(safety_scanned, start_time.elapsed()))?;
                    }
                }

                let nodes = fetch_nodes(
                    &ctx_clone,
                    BranchScope::new(&tenant_id, &repo_id, &branch),
                    &workspace,
                    chunk,
                    &scan_revision,
                )
                .await?;

                for node in nodes.into_iter().flatten() {
                    if limit.is_some_and(|lim| emitted >= lim) {
                        break;
                    }
                    if node.path == "/" {
                        continue;
                    }
                    if pseudo && !node_still_matches(&node, &property_name, &property_value) {
                        continue;
                    }

                    let node = if let Some(ref auth) = ctx_clone.auth_context {
                        let scope = PermissionScope::new(&workspace, &branch);
                        match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, Some(&scan_revision)).await {
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
                        // The driving equality, moved here from the residual
                        // filter: checked on the row as emitted (translated,
                        // field-filtered), exactly where the filter saw it.
                        if verifies_value && !json_member_equals(&translated_node, &property_name, &property_value) {
                            continue;
                        }

                        let row = node_to_row_owned(translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, None,).await?;
                        yield row;
                        emitted += 1;
                    }
                }
            }

            let short = limit.is_some_and(|lim| emitted < lim);
            if !(truncated && short) {
                break;
            }
            index_limit = None;
        }
    }))
}
