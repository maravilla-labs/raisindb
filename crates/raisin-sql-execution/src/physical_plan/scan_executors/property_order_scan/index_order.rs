// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Property index-based order scan with safety valve fallback.
//!
//! Scans the property index in sorted order for ORDER BY queries on timestamp
//! or custom properties. When an ultra-selective filter causes the index scan
//! to exhaust without producing enough rows, falls back to a filter-first
//! strategy that uses the equality predicate index.

use super::super::helpers::{extract_property_predicate_from_filter, resolve_node_for_locale};
use super::super::index_recheck::{is_pseudo_property, node_still_matches};
use super::super::node_to_row::node_to_row;
use super::super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage, StorageScope};
use std::time::Instant;

/// Execute ORDER BY property via property index scan with safety valve fallback.
pub(super) async fn execute_property_index_order_scan<S: Storage + 'static>(
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
    property_name: String,
    ascending: bool,
    target_rows: usize,
) -> Result<RowStream, ExecutionError> {
    Ok(Box::pin(try_stream! {
        // Safety valve configuration
        const SCAN_MULTIPLIER: usize = 100;

        tracing::debug!(
            "PropertyOrderScan: tenant={}, repo={}, branch={}, workspace={}",
            tenant_id, repo_id, branch, workspace
        );
        tracing::debug!(
            "PropertyOrderScan: property='{}' direction={} limit_hint={} has_filter={}",
            property_name,
            if ascending { "ASC" } else { "DESC" },
            target_rows,
            filter.is_some()
        );

        let fetch_limit = if filter.is_some() {
            let bounded_limit = if target_rows == usize::MAX {
                None
            } else {
                let max_scan = target_rows.saturating_mul(SCAN_MULTIPLIER);
                tracing::debug!(
                    "PropertyOrderScan: fetching up to {} entries (safety valve: {} * {})",
                    max_scan, target_rows, SCAN_MULTIPLIER
                );
                Some(max_scan)
            };
            bounded_limit
        } else {
            let fetch_with_buffer = if target_rows == usize::MAX {
                None
            } else {
                Some(target_rows.saturating_mul(3))
            };
            tracing::debug!(
                "PropertyOrderScan: fetching up to {} entries (no filter, 3x buffer for orphans)",
                fetch_with_buffer.map(|n| n.to_string()).unwrap_or_else(|| "unlimited".to_string())
            );
            fetch_with_buffer
        };

        // One revision for the index read, every node decode and RLS.
        let scan_revision = ctx_clone.statement_snapshot().await?;
        // A timestamp order is served by the index alone; re-check that each
        // entry is the node's CURRENT value, or an orphan entry would place the
        // node at a position its record contradicts.
        let pseudo = is_pseudo_property(&property_name);

        // The index is read in PAGES: a filter above the scan may reject
        // most entries, and stopping after one bounded fetch returned a SILENT
        // SUBSET (`LIMIT 5` found 0 of 76 matching rows). Each page re-reads
        // from the start with a 4x larger bound and skips what was already
        // consumed (one revision, so the order is stable), until enough rows
        // were emitted, the index is exhausted, or the scan budget is spent —
        // which is an error, never a short answer.
        // Seek to a range bound on the ORDER property itself
        // (`start_at >= x ORDER BY start_at`): scanning from the first entry
        // walked every earlier value before reaching the window. Text bounds
        // only, and not for a value that reads as a date (a `Date` property is
        // indexed by instant, not text); the filter still checks every row.
        let (seek_lower, seek_upper) = match &filter {
            Some(f) if !property_name.starts_with("__") => order_range_bounds(f, &property_name),
            _ => (None, None),
        };
        let seek_lower_pv = seek_lower.map(|(v, i)| (PropertyValue::String(v), i));
        let seek_upper_pv = seek_upper.map(|(v, i)| (PropertyValue::String(v), i));
        let seek = seek_lower_pv.is_some() || seek_upper_pv.is_some();

        let mut fetch_limit = fetch_limit;
        let mut consumed = 0usize;
        let mut emitted = 0usize;
        let mut scanned = 0usize;
        let start_time = Instant::now();
        let mut budget: Option<Error> = None;
        'pages: loop {
        let entries = if seek {
            storage
                .property_index()
                .scan_property_range(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &property_name,
                    seek_lower_pv.as_ref().map(|(v, i)| (v, *i)),
                    seek_upper_pv.as_ref().map(|(v, i)| (v, *i)),
                    false,
                    Some(&scan_revision),
                    ascending,
                    fetch_limit,
                )
                .await
        } else {
            storage
                .property_index()
                .scan_property(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &property_name,
                    false,
                    Some(&scan_revision),
                    ascending,
                    fetch_limit,
                )
                .await
        }
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        let entries_count = entries.len();
        tracing::debug!(
            "PropertyOrderScan: fetched {} index entries (fetch_limit={:?})",
            entries_count,
            fetch_limit
        );

        for (i, entry) in entries.iter().take(5).enumerate() {
            tracing::debug!(
                "PropertyOrderScan: entry[{}] node_id={}, value={}",
                i, entry.node_id, entry.property_value
            );
        }
        if entries_count > 5 {
            tracing::debug!("PropertyOrderScan: ... and {} more entries", entries_count - 5);
        }

        for entry in entries.into_iter().skip(consumed) {
            if target_rows != usize::MAX && emitted >= target_rows {
                break;
            }

            scanned += 1;

            if scanned > SCAN_COUNT_CEILING {
                tracing::warn!("PropertyOrderScan count limit reached: {} entries checked", scanned);
                budget = Some(super::super::scan_count_budget_exceeded(scanned, start_time.elapsed()));
                break 'pages;
            }

            if scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                tracing::warn!("PropertyOrderScan time limit reached: {:?} elapsed, {} entries checked",
                               start_time.elapsed(), scanned);
                budget = Some(super::super::scan_time_budget_exceeded(scanned, start_time.elapsed()));
                break 'pages;
            }

            let node_opt = storage
                .nodes()
                .get(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &entry.node_id,
                    Some(&scan_revision),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;

            let Some(node) = node_opt else {
                tracing::warn!(
                    "PropertyOrderScan: orphaned index entry - node '{}' not found (property='{}'), skipping",
                    entry.node_id,
                    property_name
                );
                continue;
            };

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

        consumed = entries_count;
        let exhausted = fetch_limit.is_none_or(|n| entries_count < n);
        if (target_rows != usize::MAX && emitted >= target_rows) || exhausted {
            break 'pages;
        }
        fetch_limit = fetch_limit.map(|n| n.saturating_mul(4));
        }

        // The budget ran out before the answer was complete. With nothing
        // emitted yet, an equality predicate in the filter can still produce
        // the COMPLETE answer filter-first (all matches, sorted here); any
        // other case is the budget error, never a partial result.
        let fallback_possible = emitted == 0
            && target_rows != usize::MAX
            && filter
                .as_ref()
                .and_then(extract_property_predicate_from_filter)
                .is_some();
        let budget_hit = budget.is_some();
        if let Some(err) = budget.take() {
            if !fallback_possible {
                Err(err)?;
            }
        }

        // FALLBACK: filter-first strategy for ultra-selective filters
        if budget_hit && fallback_possible {
            let remaining_needed = target_rows - emitted;
            tracing::debug!(
                "PropertyOrderScan safety valve triggered: scanned={}, emitted={}, need={}. Falling back to filter-first strategy.",
                scanned, emitted, remaining_needed
            );

            if let Some(ref filter_expr) = filter {
                if let Some((prop_name, prop_value)) = extract_property_predicate_from_filter(filter_expr) {
                    tracing::debug!(
                        "   Filter-first fallback using property: {} = {:?}",
                        prop_name, prop_value
                    );

                    let matching_node_ids = storage
                        .property_index()
                        .find_by_property(
                            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                            &prop_name,
                            &prop_value,
                            false,
                            Some(&scan_revision),
                        )
                        .await
                        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

                    tracing::debug!(
                        "   Filter-first: found {} nodes matching property predicate",
                        matching_node_ids.len()
                    );

                    let mut fallback_nodes = collect_fallback_nodes(
                        &storage,
                        &ctx_clone,
                        &tenant_id,
                        &repo_id,
                        &branch,
                        &workspace,
                        &qualifier,
                        &locales,
                        &projection,
                        filter_expr,
                        &matching_node_ids,
                        &scan_revision,
                    )
                    .await?;

                    sort_fallback_nodes(&mut fallback_nodes, &property_name, ascending);

                    for (_, row) in fallback_nodes.into_iter().take(remaining_needed) {
                        emitted += 1;
                        yield row;
                    }

                    tracing::debug!(
                        "   Filter-first fallback complete: total emitted={}",
                        emitted
                    );
                }
            }
        }

        if emitted < target_rows && target_rows != usize::MAX {
            tracing::debug!(
                "   PropertyOrderScan: emitted {} rows (requested {})",
                emitted, target_rows
            );
        }
    }))
}

/// Collect matching nodes for the filter-first fallback path.
#[allow(clippy::too_many_arguments)]
async fn collect_fallback_nodes<S: Storage + 'static>(
    storage: &std::sync::Arc<S>,
    ctx: &ExecutionContext<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    qualifier: &str,
    locales: &[String],
    projection: &Option<Vec<String>>,
    filter_expr: &raisin_sql::analyzer::TypedExpr,
    matching_node_ids: &[String],
    scan_revision: &raisin_hlc::HLC,
) -> Result<Vec<(Node, Row)>, Error> {
    let mut fallback_nodes: Vec<(Node, Row)> = Vec::new();

    for node_id in matching_node_ids {
        let node_opt = storage
            .nodes()
            .get(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                node_id,
                Some(scan_revision),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        let Some(node) = node_opt else {
            tracing::warn!(
                "PropertyOrderScan fallback: orphaned index entry - node '{}' not found, skipping",
                node_id
            );
            continue;
        };

        if node.path == "/" {
            continue;
        }

        // The fallback emits rows too, so it is subject to RLS exactly like
        // the index-order path above. It used to skip this check.
        let node = if let Some(ref auth) = ctx.auth_context {
            let scope = PermissionScope::new(workspace, branch);
            match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(
                &**storage,
                node,
                auth,
                &scope,
                tenant_id,
                repo_id,
                branch,
                Some(scan_revision),
            )
            .await
            {
                Some(n) => n,
                None => continue,
            }
        } else {
            node
        };

        let locale = locales.first().map(|s| s.as_str()).unwrap_or("en");
        let translated_node = match resolve_node_for_locale(node.clone(), ctx, locale).await? {
            Some(n) => n,
            None => continue,
        };

        let row = node_to_row(
            &translated_node,
            qualifier,
            workspace,
            projection,
            ctx,
            locale,
            None,
        )
        .await?;

        match eval_expr(filter_expr, &row) {
            Ok(raisin_sql::analyzer::Literal::Boolean(true)) => {
                fallback_nodes.push((translated_node, row));
            }
            _ => continue,
        }
    }

    Ok(fallback_nodes)
}

/// Sort fallback nodes by the ordering property.
fn sort_fallback_nodes(fallback_nodes: &mut [(Node, Row)], property_name: &str, ascending: bool) {
    fallback_nodes.sort_by(|(a, _), (b, _)| {
        let ord = match property_name {
            "__created_at" => a.created_at.cmp(&b.created_at),
            "__updated_at" => a.updated_at.cmp(&b.updated_at),
            name => {
                let a_val = a.properties.get(name);
                let b_val = b.properties.get(name);
                compare_property_values(a_val, b_val)
            }
        };
        if ascending {
            ord
        } else {
            ord.reverse()
        }
    });
}

/// Compare two optional PropertyValue references for sorting.
///
/// # Null Handling
///
/// Both `None` (missing property) and `PropertyValue::Null` (explicit null) are
/// treated equivalently and sort last (greatest). This matches SQL `NULLS LAST`
/// semantics and is intentional: in the property index, a missing property and
/// an explicit null are indistinguishable — neither has an index entry. Treating
/// them the same ensures consistent sort order regardless of whether a node
/// omits the property or sets it to null.
///
/// # Type-Specific Ordering
///
/// - **String**: lexicographic (Unicode codepoint order)
/// - **Date**: timestamp comparison via `StorageTimestamp::cmp`
/// - **Integer**: numeric `i64` comparison
/// - **Float**: IEEE 754 comparison; NaN sorts last among floats
/// - **Decimal**: exact 128-bit decimal comparison
/// - **Boolean**: `false < true`
/// - **Mixed types**: ordered by variant discriminant for deterministic results
fn compare_property_values(
    a: Option<&raisin_models::nodes::properties::PropertyValue>,
    b: Option<&raisin_models::nodes::properties::PropertyValue>,
) -> std::cmp::Ordering {
    use raisin_models::nodes::properties::PropertyValue;
    use std::cmp::Ordering;

    match (a, b) {
        // None and Null are equivalent — both sort last (see doc comment above)
        (None, None)
        | (None, Some(PropertyValue::Null))
        | (Some(PropertyValue::Null), None)
        | (Some(PropertyValue::Null), Some(PropertyValue::Null)) => Ordering::Equal,
        (None | Some(PropertyValue::Null), Some(_)) => Ordering::Greater,
        (Some(_), None | Some(PropertyValue::Null)) => Ordering::Less,
        (Some(a_val), Some(b_val)) => match (a_val, b_val) {
            (PropertyValue::String(a_s), PropertyValue::String(b_s)) => a_s.cmp(b_s),
            (PropertyValue::Date(a_d), PropertyValue::Date(b_d)) => a_d.cmp(b_d),
            (PropertyValue::Integer(a_i), PropertyValue::Integer(b_i)) => a_i.cmp(b_i),
            (PropertyValue::Float(a_f), PropertyValue::Float(b_f)) => {
                // NaN sorts last among floats
                match a_f.partial_cmp(b_f) {
                    Some(ord) => ord,
                    None => match (a_f.is_nan(), b_f.is_nan()) {
                        (true, true) => Ordering::Equal,
                        (true, false) => Ordering::Greater,
                        (false, true) => Ordering::Less,
                        (false, false) => Ordering::Equal, // shouldn't happen
                    },
                }
            }
            (PropertyValue::Decimal(a_d), PropertyValue::Decimal(b_d)) => a_d.cmp(b_d),
            (PropertyValue::Boolean(a_b), PropertyValue::Boolean(b_b)) => a_b.cmp(b_b),
            // Mixed types: order by variant discriminant for deterministic sort
            _ => {
                let a_disc = std::mem::discriminant(a_val);
                let b_disc = std::mem::discriminant(b_val);
                // Discriminant doesn't implement Ord, so compare via debug hash
                format!("{:?}", a_disc).cmp(&format!("{:?}", b_disc))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::compare_property_values;
    use raisin_models::nodes::properties::PropertyValue;
    use std::cmp::Ordering;

    #[test]
    fn test_none_none_is_equal() {
        assert_eq!(compare_property_values(None, None), Ordering::Equal);
    }

    #[test]
    fn test_none_and_null_are_equivalent() {
        // None (missing property) and Null (explicit null) are treated the same
        // because the property index has no entry for either case.
        let null = PropertyValue::Null;
        assert_eq!(compare_property_values(None, Some(&null)), Ordering::Equal);
        assert_eq!(compare_property_values(Some(&null), None), Ordering::Equal);
        assert_eq!(
            compare_property_values(Some(&null), Some(&null)),
            Ordering::Equal
        );
    }

    #[test]
    fn test_none_and_null_sort_last() {
        let val = PropertyValue::String("hello".to_string());
        // None sorts after any real value
        assert_eq!(compare_property_values(None, Some(&val)), Ordering::Greater);
        assert_eq!(compare_property_values(Some(&val), None), Ordering::Less);
        // Null sorts after any real value
        let null = PropertyValue::Null;
        assert_eq!(
            compare_property_values(Some(&null), Some(&val)),
            Ordering::Greater
        );
        assert_eq!(
            compare_property_values(Some(&val), Some(&null)),
            Ordering::Less
        );
    }

    #[test]
    fn test_string_lexicographic_order() {
        let a = PropertyValue::String("apple".to_string());
        let b = PropertyValue::String("banana".to_string());
        assert_eq!(compare_property_values(Some(&a), Some(&b)), Ordering::Less);
        assert_eq!(
            compare_property_values(Some(&b), Some(&a)),
            Ordering::Greater
        );
        assert_eq!(compare_property_values(Some(&a), Some(&a)), Ordering::Equal);
    }

    #[test]
    fn test_integer_numeric_order() {
        let a = PropertyValue::Integer(5);
        let b = PropertyValue::Integer(10);
        assert_eq!(compare_property_values(Some(&a), Some(&b)), Ordering::Less);
        assert_eq!(
            compare_property_values(Some(&b), Some(&a)),
            Ordering::Greater
        );
    }

    #[test]
    fn test_float_nan_sorts_last() {
        let nan = PropertyValue::Float(f64::NAN);
        let val = PropertyValue::Float(1.0);

        assert_eq!(
            compare_property_values(Some(&nan), Some(&val)),
            Ordering::Greater
        );
        assert_eq!(
            compare_property_values(Some(&val), Some(&nan)),
            Ordering::Less
        );
        assert_eq!(
            compare_property_values(Some(&nan), Some(&nan)),
            Ordering::Equal
        );
    }

    #[test]
    fn test_float_normal_order() {
        let a = PropertyValue::Float(1.5);
        let b = PropertyValue::Float(2.5);
        assert_eq!(compare_property_values(Some(&a), Some(&b)), Ordering::Less);
    }

    #[test]
    fn test_boolean_order() {
        let f = PropertyValue::Boolean(false);
        let t = PropertyValue::Boolean(true);
        assert_eq!(compare_property_values(Some(&f), Some(&t)), Ordering::Less);
    }

    #[test]
    fn test_mixed_types_deterministic() {
        let int_val = PropertyValue::Integer(5);
        let str_val = PropertyValue::String("5".to_string());

        let ord1 = compare_property_values(Some(&int_val), Some(&str_val));
        let ord2 = compare_property_values(Some(&int_val), Some(&str_val));
        // Must be deterministic across calls
        assert_eq!(ord1, ord2);
        // Must not be Equal (they are different types)
        assert_ne!(ord1, Ordering::Equal);
    }
}

/// Text range bounds the filter puts on `property` itself
/// (`properties->>'p' >= 'x' AND properties->>'p' < 'y'`), only from
/// top-level AND-ed comparisons with a text literal that does NOT read as a
/// date. The tightest bound on each side wins.
fn order_range_bounds(
    filter: &raisin_sql::analyzer::TypedExpr,
    property: &str,
) -> (Option<(String, bool)>, Option<(String, bool)>) {
    use raisin_sql::analyzer::{BinaryOperator, Expr, Literal};
    fn conjuncts(
        e: &raisin_sql::analyzer::TypedExpr,
        out: &mut Vec<raisin_sql::analyzer::TypedExpr>,
    ) {
        if let Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } = &e.expr
        {
            conjuncts(left, out);
            conjuncts(right, out);
        } else {
            out.push(e.clone());
        }
    }
    let looks_like_date = |v: &str| {
        v.len() >= 10
            && v.as_bytes()[4] == b'-'
            && v.as_bytes()[7] == b'-'
            && v[..4].bytes().all(|b| b.is_ascii_digit())
    };
    let mut parts = Vec::new();
    conjuncts(filter, &mut parts);
    let (mut lower, mut upper): (Option<(String, bool)>, Option<(String, bool)>) = (None, None);
    for part in &parts {
        let Expr::BinaryOp { left, op, right } = &part.expr else {
            continue;
        };
        let (key_side, lit_side, flipped) = match (
            crate::physical_plan::planner::PhysicalPlanner::match_json_key_extract(&left.expr),
            &right.expr,
        ) {
            (Some((_, _, k)), Expr::Literal(Literal::Text(v))) => (k, v.clone(), false),
            _ => match (
                crate::physical_plan::planner::PhysicalPlanner::match_json_key_extract(&right.expr),
                &left.expr,
            ) {
                (Some((_, _, k)), Expr::Literal(Literal::Text(v))) => (k, v.clone(), true),
                _ => continue,
            },
        };
        if key_side != property || looks_like_date(&lit_side) {
            continue;
        }
        let op = match (op, flipped) {
            (BinaryOperator::Gt, false) | (BinaryOperator::Lt, true) => (true, false),
            (BinaryOperator::GtEq, false) | (BinaryOperator::LtEq, true) => (true, true),
            (BinaryOperator::Lt, false) | (BinaryOperator::Gt, true) => (false, false),
            (BinaryOperator::LtEq, false) | (BinaryOperator::GtEq, true) => (false, true),
            _ => continue,
        };
        match op {
            (true, incl) => {
                if lower.as_ref().is_none_or(|(v, _)| lit_side > *v) {
                    lower = Some((lit_side, incl));
                }
            }
            (false, incl) => {
                if upper.as_ref().is_none_or(|(v, _)| lit_side < *v) {
                    upper = Some((lit_side, incl));
                }
            }
        }
    }
    (lower, upper)
}
