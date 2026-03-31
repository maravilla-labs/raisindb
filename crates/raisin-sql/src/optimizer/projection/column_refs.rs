//! Column Reference Extraction
//!
//! Recursively traverses expression trees to find all column references.

use std::collections::HashSet;

use crate::analyzer::{Expr, TypedExpr};

/// Extract column references from a typed expression
///
/// Recursively traverses the expression tree to find all column references.
/// Returns column names without table qualifiers for simplicity.
pub fn extract_column_refs(expr: &TypedExpr) -> HashSet<String> {
    match &expr.expr {
        Expr::Column { column, .. } => {
            let mut set = HashSet::new();
            set.insert(column.clone());
            set
        }

        Expr::BinaryOp { left, right, .. } => {
            let mut cols = extract_column_refs(left);
            cols.extend(extract_column_refs(right));
            cols
        }

        Expr::UnaryOp { expr, .. } => extract_column_refs(expr),

        Expr::Function { name, args, .. } => extract_function_column_refs(name, args),

        Expr::JsonExtract { object, key }
        | Expr::JsonExtractText { object, key }
        | Expr::JsonContains {
            object,
            pattern: key,
        }
        | Expr::JsonKeyExists { object, key }
        | Expr::JsonAnyKeyExists { object, keys: key }
        | Expr::JsonAllKeyExists { object, keys: key }
        | Expr::JsonRemove { object, key } => {
            let mut cols = extract_column_refs(object);
            cols.extend(extract_column_refs(key));
            cols
        }

        Expr::JsonExtractPath { object, path }
        | Expr::JsonExtractPathText { object, path }
        | Expr::JsonRemoveAtPath { object, path }
        | Expr::JsonPathMatch { object, path }
        | Expr::JsonPathExists { object, path } => {
            let mut cols = extract_column_refs(object);
            cols.extend(extract_column_refs(path));
            cols
        }

        Expr::Cast { expr, .. } => extract_column_refs(expr),

        Expr::IsNull { expr } | Expr::IsNotNull { expr } => extract_column_refs(expr),

        Expr::Between { expr, low, high } => {
            let mut cols = extract_column_refs(expr);
            cols.extend(extract_column_refs(low));
            cols.extend(extract_column_refs(high));
            cols
        }

        Expr::InList { expr, list, .. } => {
            let mut cols = extract_column_refs(expr);
            for item in list {
                cols.extend(extract_column_refs(item));
            }
            cols
        }

        Expr::InSubquery { expr, .. } => {
            // Only extract from left expression, subquery has its own scope
            extract_column_refs(expr)
        }

        Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
            let mut cols = extract_column_refs(expr);
            cols.extend(extract_column_refs(pattern));
            cols
        }

        Expr::Window {
            function,
            partition_by,
            order_by,
            ..
        } => extract_window_column_refs(function, partition_by, order_by),

        Expr::Case {
            conditions,
            else_expr,
        } => {
            let mut cols = HashSet::new();

            // Extract from all conditions and results
            for (cond, result) in conditions {
                cols.extend(extract_column_refs(cond));
                cols.extend(extract_column_refs(result));
            }

            // Extract from ELSE clause if present
            if let Some(else_result) = else_expr {
                cols.extend(extract_column_refs(else_result));
            }

            cols
        }

        // Literals don't reference columns
        Expr::Literal(_) => HashSet::new(),
    }
}

/// Extract column references from function arguments.
///
/// Special-cases `TO_JSON`/`TO_JSONB` with table references, where the semantic
/// analyzer marks the pattern by setting `column == table`. In that case, a
/// wildcard marker `"table.*"` is returned to signal that all columns are needed.
fn extract_function_column_refs(name: &str, args: &[TypedExpr]) -> HashSet<String> {
    let mut cols = HashSet::new();

    let func_name = name.to_uppercase();
    if (func_name == "TO_JSON" || func_name == "TO_JSONB") && args.len() == 1 {
        if let Expr::Column { table, column } = &args[0].expr {
            if table == column {
                // This is a table reference - mark that we need all columns
                cols.insert(format!("{}.*", table));
                return cols;
            }
        }
    }

    // Regular function argument processing
    for arg in args {
        cols.extend(extract_column_refs(arg));
    }
    cols
}

/// Extract column references from a window expression's components.
pub(crate) fn extract_window_column_refs(
    function: &crate::analyzer::WindowFunction,
    partition_by: &[TypedExpr],
    order_by: &[(TypedExpr, bool)],
) -> HashSet<String> {
    let mut cols = HashSet::new();

    // Extract from window function arguments
    match function {
        crate::analyzer::WindowFunction::Sum(expr)
        | crate::analyzer::WindowFunction::Avg(expr)
        | crate::analyzer::WindowFunction::Min(expr)
        | crate::analyzer::WindowFunction::Max(expr) => {
            cols.extend(extract_column_refs(expr));
        }
        _ => {}
    }

    // Extract from PARTITION BY
    for part_expr in partition_by {
        cols.extend(extract_column_refs(part_expr));
    }

    // Extract from ORDER BY
    for (order_expr, _) in order_by {
        cols.extend(extract_column_refs(order_expr));
    }

    cols
}
