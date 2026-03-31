//! Expression replacement logic for CSE
//!
//! This module contains the recursive expression replacement functionality
//! that transforms expressions to use CSE column references.
//!
// NOTE: File intentionally exceeds 300 lines - single exhaustive match over all Expr variants
// for recursive tree rewriting; splitting would harm readability and break the pattern.

use crate::analyzer::{Expr, TypedExpr};
use crate::optimizer::cse::hasher::ExprHasher;
use std::collections::HashMap;

/// Recursively replace common subexpressions with column references
pub(crate) fn replace_common_subexpressions(
    expr: TypedExpr,
    replacement_map: &HashMap<u64, String>,
    table_qualifier: &str,
) -> TypedExpr {
    // Check if this entire expression should be replaced
    let hash = ExprHasher::hash_expr(&expr);
    if let Some(alias) = replacement_map.get(&hash) {
        // Replace with column reference
        return TypedExpr::column(
            table_qualifier.to_string(),
            alias.clone(),
            expr.data_type.clone(),
        );
    }

    // Otherwise, recursively replace in subexpressions
    let data_type = expr.data_type.clone();
    let new_expr = match expr.expr {
        Expr::BinaryOp { left, op, right } => Expr::BinaryOp {
            left: Box::new(replace_common_subexpressions(
                *left,
                replacement_map,
                table_qualifier,
            )),
            op,
            right: Box::new(replace_common_subexpressions(
                *right,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op,
            expr: Box::new(replace_common_subexpressions(
                *inner,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::Cast {
            expr: inner,
            target_type,
        } => Expr::Cast {
            expr: Box::new(replace_common_subexpressions(
                *inner,
                replacement_map,
                table_qualifier,
            )),
            target_type,
        },

        Expr::Function {
            name,
            args,
            signature,
            filter,
        } => Expr::Function {
            name,
            args: args
                .into_iter()
                .map(|arg| replace_common_subexpressions(arg, replacement_map, table_qualifier))
                .collect(),
            signature,
            filter: filter.map(|f| {
                Box::new(replace_common_subexpressions(
                    *f,
                    replacement_map,
                    table_qualifier,
                ))
            }),
        },

        Expr::IsNull { expr: inner } => Expr::IsNull {
            expr: Box::new(replace_common_subexpressions(
                *inner,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::IsNotNull { expr: inner } => Expr::IsNotNull {
            expr: Box::new(replace_common_subexpressions(
                *inner,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::Between { expr, low, high } => Expr::Between {
            expr: Box::new(replace_common_subexpressions(
                *expr,
                replacement_map,
                table_qualifier,
            )),
            low: Box::new(replace_common_subexpressions(
                *low,
                replacement_map,
                table_qualifier,
            )),
            high: Box::new(replace_common_subexpressions(
                *high,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::InList {
            expr,
            list,
            negated,
        } => Expr::InList {
            expr: Box::new(replace_common_subexpressions(
                *expr,
                replacement_map,
                table_qualifier,
            )),
            list: list
                .into_iter()
                .map(|item| replace_common_subexpressions(item, replacement_map, table_qualifier))
                .collect(),
            negated,
        },

        Expr::InSubquery {
            expr,
            subquery,
            subquery_type,
            negated,
        } => Expr::InSubquery {
            expr: Box::new(replace_common_subexpressions(
                *expr,
                replacement_map,
                table_qualifier,
            )),
            subquery,
            subquery_type,
            negated,
        },

        Expr::Like {
            expr,
            pattern,
            negated,
        } => Expr::Like {
            expr: Box::new(replace_common_subexpressions(
                *expr,
                replacement_map,
                table_qualifier,
            )),
            pattern: Box::new(replace_common_subexpressions(
                *pattern,
                replacement_map,
                table_qualifier,
            )),
            negated,
        },

        Expr::ILike {
            expr,
            pattern,
            negated,
        } => Expr::ILike {
            expr: Box::new(replace_common_subexpressions(
                *expr,
                replacement_map,
                table_qualifier,
            )),
            pattern: Box::new(replace_common_subexpressions(
                *pattern,
                replacement_map,
                table_qualifier,
            )),
            negated,
        },

        Expr::JsonExtract { object, key } => Expr::JsonExtract {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            key: Box::new(replace_common_subexpressions(
                *key,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonExtractText { object, key } => Expr::JsonExtractText {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            key: Box::new(replace_common_subexpressions(
                *key,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonContains { object, pattern } => Expr::JsonContains {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            pattern: Box::new(replace_common_subexpressions(
                *pattern,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonKeyExists { object, key } => Expr::JsonKeyExists {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            key: Box::new(replace_common_subexpressions(
                *key,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonAnyKeyExists { object, keys } => Expr::JsonAnyKeyExists {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            keys: Box::new(replace_common_subexpressions(
                *keys,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonAllKeyExists { object, keys } => Expr::JsonAllKeyExists {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            keys: Box::new(replace_common_subexpressions(
                *keys,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonExtractPath { object, path } => Expr::JsonExtractPath {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            path: Box::new(replace_common_subexpressions(
                *path,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonExtractPathText { object, path } => Expr::JsonExtractPathText {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            path: Box::new(replace_common_subexpressions(
                *path,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonRemove { object, key } => Expr::JsonRemove {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            key: Box::new(replace_common_subexpressions(
                *key,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonRemoveAtPath { object, path } => Expr::JsonRemoveAtPath {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            path: Box::new(replace_common_subexpressions(
                *path,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonPathMatch { object, path } => Expr::JsonPathMatch {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            path: Box::new(replace_common_subexpressions(
                *path,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::JsonPathExists { object, path } => Expr::JsonPathExists {
            object: Box::new(replace_common_subexpressions(
                *object,
                replacement_map,
                table_qualifier,
            )),
            path: Box::new(replace_common_subexpressions(
                *path,
                replacement_map,
                table_qualifier,
            )),
        },

        Expr::Window {
            function,
            partition_by,
            order_by,
            frame,
        } => {
            let new_function = replace_window_function(function, replacement_map, table_qualifier);

            Expr::Window {
                function: new_function,
                partition_by: partition_by
                    .into_iter()
                    .map(|e| replace_common_subexpressions(e, replacement_map, table_qualifier))
                    .collect(),
                order_by: order_by
                    .into_iter()
                    .map(|(e, desc)| {
                        (
                            replace_common_subexpressions(e, replacement_map, table_qualifier),
                            desc,
                        )
                    })
                    .collect(),
                frame,
            }
        }

        Expr::Case {
            conditions,
            else_expr,
        } => Expr::Case {
            conditions: conditions
                .into_iter()
                .map(|(cond, result)| {
                    (
                        replace_common_subexpressions(cond, replacement_map, table_qualifier),
                        replace_common_subexpressions(result, replacement_map, table_qualifier),
                    )
                })
                .collect(),
            else_expr: else_expr.map(|e| {
                Box::new(replace_common_subexpressions(
                    *e,
                    replacement_map,
                    table_qualifier,
                ))
            }),
        },

        // Leaf nodes - no replacement needed
        Expr::Literal(_) | Expr::Column { .. } => expr.expr,
    };

    TypedExpr::new(new_expr, data_type)
}

/// Replace common subexpressions in window functions
fn replace_window_function(
    function: crate::analyzer::WindowFunction,
    replacement_map: &HashMap<u64, String>,
    table_qualifier: &str,
) -> crate::analyzer::WindowFunction {
    match function {
        crate::analyzer::WindowFunction::Sum(e) => crate::analyzer::WindowFunction::Sum(Box::new(
            replace_common_subexpressions(*e, replacement_map, table_qualifier),
        )),
        crate::analyzer::WindowFunction::Avg(e) => crate::analyzer::WindowFunction::Avg(Box::new(
            replace_common_subexpressions(*e, replacement_map, table_qualifier),
        )),
        crate::analyzer::WindowFunction::Min(e) => crate::analyzer::WindowFunction::Min(Box::new(
            replace_common_subexpressions(*e, replacement_map, table_qualifier),
        )),
        crate::analyzer::WindowFunction::Max(e) => crate::analyzer::WindowFunction::Max(Box::new(
            replace_common_subexpressions(*e, replacement_map, table_qualifier),
        )),
        other => other,
    }
}
