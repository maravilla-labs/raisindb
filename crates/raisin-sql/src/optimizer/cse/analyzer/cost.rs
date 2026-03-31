//! Cost estimation, volatility detection, and extractability checks
//!
//! This module determines whether an expression is worth extracting by CSE,
//! based on computational cost, determinism (volatility), and structural checks.

use crate::analyzer::TypedExpr;

/// Estimate the computational cost of evaluating an expression
///
/// Returns a cost score where higher values indicate more expensive operations.
/// This helps CSE avoid extracting cheap expressions that cost more to materialize
/// than to recompute.
///
/// Cost guidelines:
/// - 0-5: Very cheap (literals, columns, simple arithmetic)
/// - 10-20: Moderate (casts, comparisons, simple functions)
/// - 30-50: Expensive (JSON operations, complex functions, LIKE)
/// - 100+: Very expensive (window functions, complex aggregates)
fn estimate_cost(expr: &TypedExpr) -> u32 {
    use crate::analyzer::Expr;

    match &expr.expr {
        // Leaf nodes - essentially free
        Expr::Literal(_) => 1,
        Expr::Column { .. } => 2,

        // Simple arithmetic - very cheap
        Expr::BinaryOp {
            left, right, op, ..
        } => {
            let base_cost = match op {
                crate::analyzer::BinaryOperator::Add
                | crate::analyzer::BinaryOperator::Subtract
                | crate::analyzer::BinaryOperator::Multiply => 3,
                crate::analyzer::BinaryOperator::Divide
                | crate::analyzer::BinaryOperator::Modulo => 5,
                _ => 4, // Comparisons, logical ops
            };
            base_cost + estimate_cost(left) + estimate_cost(right)
        }

        Expr::UnaryOp { expr: inner, .. } => 3 + estimate_cost(inner),

        Expr::Cast { expr: inner, .. } => 10 + estimate_cost(inner),

        // NULL checks - cheap
        Expr::IsNull { expr: inner } | Expr::IsNotNull { expr: inner } => 5 + estimate_cost(inner),

        // Pattern matching - moderate to expensive
        Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
            30 + estimate_cost(expr) + estimate_cost(pattern)
        }

        // Range and list operations
        Expr::Between { expr, low, high } => {
            15 + estimate_cost(expr) + estimate_cost(low) + estimate_cost(high)
        }

        Expr::InList { expr, list, .. } => {
            let list_cost: u32 = list.iter().map(estimate_cost).sum();
            20 + estimate_cost(expr) + list_cost
        }

        Expr::InSubquery { expr, .. } => {
            // Subquery execution is very expensive
            100 + estimate_cost(expr)
        }

        // JSON operations - expensive
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
            50 + estimate_cost(object) + estimate_cost(key)
        }

        // Functions - depends on the function
        Expr::Function { args, filter, .. } => {
            let args_cost: u32 = args.iter().map(estimate_cost).sum();
            let filter_cost = filter.as_ref().map(|f| estimate_cost(f)).unwrap_or(0);
            30 + args_cost + filter_cost
        }

        // Window functions - very expensive
        Expr::Window {
            partition_by,
            order_by,
            ..
        } => {
            let func_cost = 100;
            let partition_cost: u32 = partition_by.iter().map(estimate_cost).sum();
            let order_cost: u32 = order_by.iter().map(|(e, _)| estimate_cost(e)).sum();
            func_cost + partition_cost + order_cost
        }

        // CASE expressions - moderate
        Expr::Case {
            conditions,
            else_expr,
        } => {
            let conditions_cost: u32 = conditions
                .iter()
                .map(|(c, r)| estimate_cost(c) + estimate_cost(r))
                .sum();
            let else_cost = else_expr.as_ref().map(|e| estimate_cost(e)).unwrap_or(0);
            20 + conditions_cost + else_cost
        }
    }
}

/// Check if an expression contains volatile (non-deterministic) functions
///
/// CRITICAL: Volatile functions must NEVER be extracted by CSE because they
/// should return different values on each invocation.
///
/// Examples of volatile functions:
/// - random() - Must return different values each time
/// - now(), clock_timestamp() - Returns current time
/// - uuid_generate_v4() - Generates unique IDs
///
/// Bug example without this check:
/// ```sql
/// SELECT random() as r1, random() as r2
/// -- Without check: CSE extracts to __cse_0, both columns get SAME value (WRONG!)
/// -- With check: Each random() call returns different value (CORRECT)
/// ```
fn is_volatile(expr: &TypedExpr) -> bool {
    use crate::analyzer::Expr;

    match &expr.expr {
        // Function calls - check if deterministic
        Expr::Function {
            signature,
            args,
            filter,
            ..
        } => {
            // If function itself is non-deterministic, expression is volatile
            if !signature.is_deterministic {
                return true;
            }

            // Recursively check arguments
            for arg in args {
                if is_volatile(arg) {
                    return true;
                }
            }

            if let Some(f) = filter {
                if is_volatile(f) {
                    return true;
                }
            }

            false
        }

        // Recursively check all child expressions
        Expr::BinaryOp { left, right, .. } => is_volatile(left) || is_volatile(right),

        Expr::UnaryOp { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::IsNull { expr: inner }
        | Expr::IsNotNull { expr: inner } => is_volatile(inner),

        Expr::Between { expr, low, high } => {
            is_volatile(expr) || is_volatile(low) || is_volatile(high)
        }

        Expr::InList { expr, list, .. } => is_volatile(expr) || list.iter().any(is_volatile),

        Expr::InSubquery { expr: _expr, .. } => {
            // Subqueries are considered volatile for CSE purposes
            // (their result depends on database state)
            true
        }

        Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
            is_volatile(expr) || is_volatile(pattern)
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
        | Expr::JsonPathExists { object, path: key } => is_volatile(object) || is_volatile(key),

        Expr::Window {
            function,
            partition_by,
            order_by,
            ..
        } => {
            // Window functions themselves can be volatile
            let func_volatile = match function {
                crate::analyzer::WindowFunction::Sum(e)
                | crate::analyzer::WindowFunction::Avg(e)
                | crate::analyzer::WindowFunction::Min(e)
                | crate::analyzer::WindowFunction::Max(e) => is_volatile(e),
                _ => false, // RowNumber, Rank, DenseRank, Count are deterministic
            };

            func_volatile
                || partition_by.iter().any(is_volatile)
                || order_by.iter().any(|(e, _)| is_volatile(e))
        }

        Expr::Case {
            conditions: _,
            else_expr: _,
        } => {
            // CRITICAL: Cannot extract from CASE branches due to short-circuit evaluation
            // Example: CASE WHEN false THEN 1/0 ELSE 1 END
            // If we extract 1/0, it will execute and cause division by zero
            // even though the branch should never execute!
            //
            // Therefore, treat ANY expression inside CASE as volatile
            // to prevent extraction across branch boundaries.
            true // Always volatile - prevent CSE from extracting CASE internals
        }

        // Leaf nodes are deterministic
        Expr::Literal(_) | Expr::Column { .. } => false,
    }
}

/// Determine if an expression is worth extracting based on cost and complexity
///
/// Extraction is only worthwhile if:
/// 1. The expression is not trivial (literal/column)
/// 2. The expression is deterministic (not volatile)
/// 3. The computational cost exceeds the materialization overhead
/// 4. The expression is safe to lift (not inside CASE branches)
///
/// Minimum extraction cost threshold: 10
/// (Cheap operations like `a + 1` cost ~4, not worth extracting)
pub(super) fn is_extractable(expr: &TypedExpr) -> bool {
    const MIN_EXTRACTION_COST: u32 = 10;

    use crate::analyzer::Expr;

    // Never extract leaf nodes
    if matches!(expr.expr, Expr::Literal(_) | Expr::Column { .. }) {
        return false;
    }

    // Never extract simple Cast(Column) patterns
    // These are cheap operations (type annotations) and extracting them causes
    // column resolution issues when the CSE column reference has wrong/empty
    // table qualifiers. This fixes the bug where multiple $.properties.* or
    // JSON_GET expressions return NULL.
    if let Expr::Cast { expr: inner, .. } = &expr.expr {
        if matches!(inner.expr, Expr::Column { .. }) {
            return false;
        }
    }

    // CRITICAL: Never extract volatile (non-deterministic) expressions
    // This prevents the random() bug where CSE would cache random values
    if is_volatile(expr) {
        return false;
    }

    // Only extract if cost justifies materialization overhead
    estimate_cost(expr) >= MIN_EXTRACTION_COST
}
