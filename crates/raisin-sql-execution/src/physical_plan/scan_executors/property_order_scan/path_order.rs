// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `ORDER BY path` scan, paged in path-index order.
//!
//! `ORDER BY path` is byte-wise order of the path STRING. That is neither tree
//! order (`/a-b` sorts before `/a/x`, since `-` is below `/`) nor editorial
//! order (`__tree_order`), so the scan must not walk the tree. It pages through
//! the path index instead, whose key order is exactly path order, via
//! [`NodeRepository::scan_by_path_order_page`].
//!
//! This used to be a depth-first walk of ORDERED_CHILDREN that began by loading
//! the workspace root node. A workspace has no stored root node, so the walk
//! found nothing and every `ORDER BY path LIMIT n` without a path-prefix
//! predicate returned zero rows. Where a root did exist, it produced editorial
//! order, which broke `path > $cursor` keyset paging.

use super::super::helpers::{resolve_node_for_locale, scan_needs_properties};
use super::super::node_to_row::node_to_row;
use super::super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use async_stream::try_stream;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};
use std::time::Instant;

/// Index entries requested per storage page. A LIMIT smaller than this is
/// still read as one page: rows can be dropped above the seek (RLS, filter,
/// locale), and a too-small page would just mean another round trip.
const MIN_PAGE: usize = 32;
const MAX_PAGE: usize = 1024;

/// Execute ORDER BY path by paging through the path index.
pub(super) async fn execute_path_order_scan<S: Storage + 'static>(
    storage: std::sync::Arc<S>,
    ctx_clone: ExecutionContext<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    qualifier: String,
    locales: Vec<String>,
    projection: Option<Vec<String>>,
    filter: Option<TypedExpr>,
    ascending: bool,
    target_rows: usize,
) -> Result<RowStream, ExecutionError> {
    let descending = !ascending;
    let max_revision = ctx_clone.max_revision;
    // The filter is evaluated against the row, and may read properties.
    let skip_properties = filter.is_none() && !scan_needs_properties(&projection, &ctx_clone);
    let list_options = move || {
        let options = match max_revision.as_ref() {
            Some(rev) => ListOptions::at_revision(*rev),
            None => ListOptions::for_sql(),
        };
        if skip_properties {
            options.without_properties()
        } else {
            options
        }
    };
    let page_size = target_rows.clamp(MIN_PAGE, MAX_PAGE);
    // A `path > 'x'` (or, descending, `path < 'x'`) conjunct is a keyset cursor:
    // seek to it rather than walking every smaller path. The filter is still
    // applied to every row, so this only changes where the walk starts.
    let mut cursor = filter
        .as_ref()
        .and_then(|f| path_keyset_cursor(f, descending));

    Ok(Box::pin(try_stream! {
        tracing::debug!(
            "   PropertyOrderScan: path ordering via path index direction={} limit_hint={} cursor={:?}",
            if ascending { "ASC" } else { "DESC" },
            target_rows,
            cursor
        );

        let start_time = Instant::now();
        let mut scanned = 0usize;
        let mut emitted = 0usize;

        'pages: loop {
            let page = storage
                .nodes()
                .scan_by_path_order_page(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    cursor.as_deref(),
                    page_size,
                    descending,
                    list_options(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;

            for node in page.nodes {
                scanned += 1;
                if scanned > SCAN_COUNT_CEILING {
                    Err(super::super::scan_count_budget_exceeded(scanned, start_time.elapsed()))?;
                }
                if scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                    Err(super::super::scan_time_budget_exceeded(scanned, start_time.elapsed()))?;
                }

                if node.path == "/" {
                    continue;
                }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, max_revision.as_ref()).await {
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
                            Ok(Literal::Boolean(true)) => {}
                            Ok(Literal::Boolean(false)) | Ok(Literal::Null) => continue,
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
                        break 'pages;
                    }
                }
            }

            match page.next_after {
                Some(next) => cursor = Some(next),
                None => break 'pages,
            }
        }
    }))
}

/// The exclusive keyset bound a path-ordered walk can seek to: the literal of a
/// top-level `path > 'x'` conjunct when ascending, `path < 'x'` when
/// descending (either operand order). Inclusive bounds are left to the filter
/// rather than approximated.
fn path_keyset_cursor(filter: &TypedExpr, descending: bool) -> Option<String> {
    let wanted = if descending {
        BinaryOperator::Lt
    } else {
        BinaryOperator::Gt
    };
    let mirrored = if descending {
        BinaryOperator::Gt
    } else {
        BinaryOperator::Lt
    };
    let mut best: Option<String> = None;
    let mut pending = vec![filter];
    while let Some(expr) = pending.pop() {
        let Expr::BinaryOp { left, op, right } = &expr.expr else {
            continue;
        };
        if *op == BinaryOperator::And {
            pending.push(left);
            pending.push(right);
            continue;
        }
        let bound = match (&left.expr, &right.expr) {
            (Expr::Column { column, .. }, Expr::Literal(Literal::Text(v)))
                if *op == wanted && is_path_column(column) =>
            {
                v
            }
            (Expr::Literal(Literal::Text(v)), Expr::Column { column, .. })
                if *op == mirrored && is_path_column(column) =>
            {
                v
            }
            _ => continue,
        };
        // Two bounds in one AND: the tighter one wins.
        let tighter = match &best {
            None => true,
            Some(b) if descending => bound < b,
            Some(b) => bound > b,
        };
        if tighter {
            best = Some(bound.clone());
        }
    }
    best
}

fn is_path_column(column: &str) -> bool {
    column
        .rsplit('.')
        .next()
        .is_some_and(|c| c.eq_ignore_ascii_case("path"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::DataType;

    fn col(name: &str) -> TypedExpr {
        TypedExpr::new(
            Expr::Column {
                table: "t".into(),
                column: name.into(),
            },
            DataType::Text,
        )
    }

    fn text(v: &str) -> TypedExpr {
        TypedExpr::new(Expr::Literal(Literal::Text(v.into())), DataType::Text)
    }

    fn bin(left: TypedExpr, op: BinaryOperator, right: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            },
            DataType::Boolean,
        )
    }

    #[test]
    fn ascending_seeks_on_greater_than() {
        let f = bin(col("path"), BinaryOperator::Gt, text("/a"));
        assert_eq!(path_keyset_cursor(&f, false).as_deref(), Some("/a"));
        assert_eq!(path_keyset_cursor(&f, true), None);
    }

    #[test]
    fn descending_seeks_on_less_than_either_operand_order() {
        let f = bin(text("/m"), BinaryOperator::Gt, col("path"));
        assert_eq!(path_keyset_cursor(&f, true).as_deref(), Some("/m"));
        assert_eq!(path_keyset_cursor(&f, false), None);
    }

    #[test]
    fn inclusive_bounds_and_other_columns_do_not_seek() {
        let ge = bin(col("path"), BinaryOperator::GtEq, text("/a"));
        let name = bin(col("name"), BinaryOperator::Gt, text("/a"));
        assert_eq!(path_keyset_cursor(&ge, false), None);
        assert_eq!(path_keyset_cursor(&name, false), None);
    }

    #[test]
    fn found_inside_and_and_tighter_bound_wins() {
        let f = bin(
            bin(col("path"), BinaryOperator::Gt, text("/a")),
            BinaryOperator::And,
            bin(
                bin(col("node_type"), BinaryOperator::Eq, text("x")),
                BinaryOperator::And,
                bin(col("t.path"), BinaryOperator::Gt, text("/c")),
            ),
        );
        assert_eq!(path_keyset_cursor(&f, false).as_deref(), Some("/c"));
    }

    #[test]
    fn a_bound_under_or_is_not_a_cursor() {
        let f = bin(
            bin(col("path"), BinaryOperator::Gt, text("/a")),
            BinaryOperator::Or,
            bin(col("name"), BinaryOperator::Eq, text("x")),
        );
        assert_eq!(path_keyset_cursor(&f, false), None);
    }
}
