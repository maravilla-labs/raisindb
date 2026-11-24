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
use super::index_recheck::{is_pseudo_property, node_still_matches};
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

    // Convert the planner's bounds to the storage layer's. Timestamp bounds
    // arrive as decimal MICROSECONDS (see `planner::scan_planning::
    // timestamp_bounds`), already rounded so the scan is exact; the storage
    // reader encodes a `PropertyValue::Integer` for these properties as the
    // same 8-byte big-endian micros its keys use. Everything else is text.
    let parse_bound =
        |bound: &Option<(String, bool)>, prop_name: &str| -> Option<(PropertyValue, bool)> {
            bound.as_ref().map(|(val, inclusive)| {
                if prop_name == "__created_at" || prop_name == "__updated_at" {
                    if let Ok(micros) = val.parse::<i64>() {
                        return (PropertyValue::Integer(micros), *inclusive);
                    }
                }
                (PropertyValue::String(val.clone()), *inclusive)
            })
        };

    let lower_pv = parse_bound(&lower_bound, &property_name);
    let upper_pv = parse_bound(&upper_bound, &property_name);
    // A `Date` property is indexed by its instant (`hash_property_value`:
    // zero-padded nanoseconds), not its text, so a text bound like
    // '2026-11-17' compared lexicographically against those digits matched
    // NOTHING. When a bound reads as a date, the instant range is scanned too
    // (widened by a day for offsets); the planner keeps the range predicate
    // in the residual filter for custom properties, so each row is still
    // judged by the exact text comparison `->>` makes.
    let date_bounds = if property_name.starts_with("__") {
        None
    } else {
        date_range_bounds(lower_bound.as_ref(), upper_bound.as_ref())
    };

    tracing::debug!(
        "PropertyRangeScan: {} (lower={:?}, upper={:?}) {} limit={:?}",
        property_name,
        lower_bound,
        upper_bound,
        if ascending { "ASC" } else { "DESC" },
        limit
    );

    Ok(Box::pin(try_stream! {
        // One revision for the index read and every node decode.
        let scan_revision = ctx_clone.statement_snapshot().await?;
        // `created_at` / `updated_at` ranges are answered by the index alone,
        // so each candidate is re-checked against the decoded node.
        let pseudo = is_pseudo_property(&property_name);

        let mut emitted = 0usize;
        // First ask for `limit` entries; if that page came back full but
        // dropped candidates left the caller short, continue with an unlimited
        // pass past the ids already seen (the order is the same, so the rows
        // already emitted are its prefix).
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut index_limit = limit;
        loop {
            let mut entries = storage
                .property_index()
                .scan_property_range(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &property_name,
                    lower_pv.as_ref().map(|(v, i)| (v, *i)),
                    upper_pv.as_ref().map(|(v, i)| (v, *i)),
                    false,
                    Some(&scan_revision),
                    ascending,
                    index_limit,
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;
            let mut truncated = index_limit.is_some_and(|l| entries.len() >= l);
            if let Some((date_lower, date_upper)) = &date_bounds {
                let dated = storage
                    .property_index()
                    .scan_property_range(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        &property_name,
                        date_lower.as_ref().map(|v| (v, true)),
                        date_upper.as_ref().map(|v| (v, true)),
                        false,
                        Some(&scan_revision),
                        ascending,
                        index_limit,
                    )
                    .await
                    .map_err(|e| ExecutionError::Backend(e.to_string()))?;
                truncated |= index_limit.is_some_and(|l| dated.len() >= l);
                entries.extend(dated);
            }

            for entry in entries {
                if emitted >= limit_count {
                    break;
                }
                if !seen.insert(entry.node_id.clone()) {
                    continue;
                }

                let node_opt = super::helpers::row_node(
                    &*storage,
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    raisin_storage::NodeLocator::Id(entry.node_id.clone()),
                    Some(&scan_revision),
                )
                .await
                    .map_err(|e| ExecutionError::Backend(e.to_string()))?;

                if let Some(node) = node_opt {
                    if node.path == "/" {
                        continue;
                    }
                    if pseudo && !node_still_matches(&node, &property_name, &entry.property_value) {
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
                            None,
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

            if !(truncated && emitted < limit_count) {
                break;
            }
            index_limit = None;
        }
    }))
}

/// A text bound that reads as a date or date-time, as an instant (UTC).
fn parse_date_bound(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    let t = text.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(n) = NaiveDateTime::parse_from_str(t, fmt) {
            return Some(Utc.from_utc_datetime(&n));
        }
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|n| Utc.from_utc_datetime(&n))
}

/// Instant bounds for a `Date` property's index when the text bounds read as
/// dates: widened by a day each way (a stored value may carry any offset; the
/// residual filter makes the exact text comparison). `None` when neither
/// bound is a date, or one bound is present but not a date.
#[allow(clippy::type_complexity)]
fn date_range_bounds(
    lower: Option<&(String, bool)>,
    upper: Option<&(String, bool)>,
) -> Option<(Option<PropertyValue>, Option<PropertyValue>)> {
    let day = chrono::Duration::days(1);
    let conv = |b: Option<&(String, bool)>,
                shift: chrono::Duration|
     -> Result<Option<PropertyValue>, ()> {
        match b {
            None => Ok(None),
            Some((text, _)) => parse_date_bound(text)
                .map(|dt| Some(PropertyValue::Date((dt + shift).into())))
                .ok_or(()),
        }
    };
    if lower.is_none() && upper.is_none() {
        return None;
    }
    let lo = conv(lower, -day).ok()?;
    let hi = conv(upper, day).ok()?;
    Some((lo, hi))
}

#[cfg(test)]
mod date_bound_tests {
    use super::{date_range_bounds, parse_date_bound};

    #[test]
    fn dates_and_date_times_read_as_instants() {
        for t in [
            "2026-11-17",
            "2026-11-17T00:00:00Z",
            "2026-11-17T00:00:00+00:00",
            "2026-11-17T00:00",
        ] {
            assert_eq!(
                parse_date_bound(t).unwrap().to_rfc3339(),
                "2026-11-17T00:00:00+00:00",
                "{t}"
            );
        }
        assert!(parse_date_bound("open").is_none());
        assert!(date_range_bounds(Some(&("M".into(), true)), None).is_none());
        assert!(date_range_bounds(Some(&("2026-11-17".into(), true)), None).is_some());
    }
}
