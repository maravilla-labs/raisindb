//! Expression tree traversal and subexpression collection
//!
//! This module provides the iterative traversal logic for collecting subexpressions
//! and their frequencies, plus structural equality checks for hash collision handling.

use crate::analyzer::TypedExpr;
use crate::optimizer::cse::arena::{ExprId, ExpressionArena};
use crate::optimizer::cse::hasher::ExprHasher;
use std::collections::HashMap;

/// Check if two expressions are structurally equal
///
/// This performs a deep equality check to handle hash collisions correctly.
/// Two expressions are structurally equal if they have the same structure,
/// operators, and values, regardless of memory location.
pub(super) fn expressions_equal(a: &TypedExpr, b: &TypedExpr) -> bool {
    // Check data types match
    if a.data_type != b.data_type {
        return false;
    }

    // Check expression structure matches
    // For simplicity, we use debug format comparison
    // In production, you might want a more efficient deep equality check
    format!("{:?}", a.expr) == format!("{:?}", b.expr)
}

/// Iteratively collect all subexpressions and their frequencies
///
/// Uses a stack-based approach to avoid stack overflow on deeply nested expressions.
/// This is critical for production use where queries may have very deep expression trees
/// (e.g., 1000+ levels of nesting from programmatically generated SQL).
///
/// Hash collision handling: Uses Vec to store multiple expressions with the same hash,
/// performing structural equality checks to avoid incorrect CSE.
pub(super) fn collect_subexpressions(
    expr: &TypedExpr,
    arena: &mut ExpressionArena,
    frequency_map: &mut HashMap<u64, Vec<(ExprId, usize)>>,
) {
    use crate::analyzer::Expr;

    // Stack for iterative traversal: stores references to expressions to process
    let mut stack: Vec<&TypedExpr> = vec![expr];

    while let Some(current) = stack.pop() {
        // Compute hash for this expression
        let hash = ExprHasher::hash_expr(current);

        // Handle hash collisions with structural equality check
        let entry = frequency_map.entry(hash).or_default();

        // Check if we've seen this exact expression before
        let mut found = false;
        for (expr_id, count) in entry.iter_mut() {
            if expressions_equal(arena.get(*expr_id), current) {
                *count += 1;
                found = true;
                break;
            }
        }

        // If not found, add new entry
        if !found {
            let expr_id = arena.add(current.clone());
            entry.push((expr_id, 1));
        }

        // Push child expressions onto stack for processing
        match &current.expr {
            Expr::BinaryOp { left, right, .. } => {
                stack.push(right);
                stack.push(left);
            }

            Expr::UnaryOp { expr: inner, .. }
            | Expr::Cast { expr: inner, .. }
            | Expr::IsNull { expr: inner }
            | Expr::IsNotNull { expr: inner } => {
                stack.push(inner);
            }

            Expr::Function { args, filter, .. } => {
                if let Some(filter_expr) = filter {
                    stack.push(filter_expr);
                }
                for arg in args.iter().rev() {
                    stack.push(arg);
                }
            }

            Expr::Between { expr, low, high } => {
                stack.push(high);
                stack.push(low);
                stack.push(expr);
            }

            Expr::InList { expr, list, .. } => {
                for item in list.iter().rev() {
                    stack.push(item);
                }
                stack.push(expr);
            }

            Expr::InSubquery { expr, .. } => {
                // Only push the left expression, subquery has its own scope
                stack.push(expr);
            }

            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                stack.push(pattern);
                stack.push(expr);
            }

            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonContains {
                object,
                pattern: key,
            }
            | Expr::JsonKeyExists { object, key }
            | Expr::JsonAnyKeyExists { object, keys: key }
            | Expr::JsonAllKeyExists { object, keys: key }
            | Expr::JsonExtractPath { object, path: key }
            | Expr::JsonExtractPathText { object, path: key }
            | Expr::JsonRemove { object, key }
            | Expr::JsonRemoveAtPath { object, path: key }
            | Expr::JsonPathMatch { object, path: key }
            | Expr::JsonPathExists { object, path: key } => {
                stack.push(key);
                stack.push(object);
            }

            Expr::Window {
                function,
                partition_by,
                order_by,
                ..
            } => {
                // Process window function arguments
                match function {
                    crate::analyzer::WindowFunction::Sum(e)
                    | crate::analyzer::WindowFunction::Avg(e)
                    | crate::analyzer::WindowFunction::Min(e)
                    | crate::analyzer::WindowFunction::Max(e) => {
                        stack.push(e);
                    }
                    _ => {}
                }

                for (expr, _) in order_by.iter().rev() {
                    stack.push(expr);
                }
                for expr in partition_by.iter().rev() {
                    stack.push(expr);
                }
            }

            Expr::Case {
                conditions,
                else_expr,
            } => {
                if let Some(else_e) = else_expr {
                    stack.push(else_e);
                }
                for (cond, result) in conditions.iter().rev() {
                    stack.push(result);
                    stack.push(cond);
                }
            }

            // Leaf nodes - no children to process
            Expr::Literal(_) | Expr::Column { .. } => {}
        }
    }
}
