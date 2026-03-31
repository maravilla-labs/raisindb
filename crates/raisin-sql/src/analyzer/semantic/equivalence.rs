//! Expression equivalence checking
//!
//! This module provides utilities for checking if two expressions are semantically
//! equivalent, which is used for GROUP BY validation and query optimization.

use super::super::typed_expr::{Expr, Literal, TypedExpr};

/// Check if two typed expressions are equivalent
///
/// Two expressions are considered equivalent if they have the same structure
/// and produce the same value. This is used for GROUP BY validation to check
/// if expressions in SELECT match GROUP BY expressions.
pub(super) fn expressions_equivalent(a: &TypedExpr, b: &TypedExpr) -> bool {
    exprs_equivalent_inner(&a.expr, &b.expr)
}

/// Inner recursive helper for expression equivalence checking
fn exprs_equivalent_inner(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        // Literals - compare values
        (Expr::Literal(lit_a), Expr::Literal(lit_b)) => literals_equivalent(lit_a, lit_b),

        // Column references - compare table and column names
        (
            Expr::Column {
                table: table_a,
                column: col_a,
            },
            Expr::Column {
                table: table_b,
                column: col_b,
            },
        ) => table_a == table_b && col_a == col_b,

        // Function calls - compare name and arguments
        (
            Expr::Function {
                name: name_a,
                args: args_a,
                ..
            },
            Expr::Function {
                name: name_b,
                args: args_b,
                ..
            },
        ) => {
            // Case-insensitive function name comparison
            name_a.eq_ignore_ascii_case(name_b)
                && args_a.len() == args_b.len()
                && args_a
                    .iter()
                    .zip(args_b.iter())
                    .all(|(arg_a, arg_b)| expressions_equivalent(arg_a, arg_b))
        }

        // Binary operators - compare operator and operands
        (
            Expr::BinaryOp {
                left: left_a,
                op: op_a,
                right: right_a,
            },
            Expr::BinaryOp {
                left: left_b,
                op: op_b,
                right: right_b,
            },
        ) => {
            op_a == op_b
                && expressions_equivalent(left_a, left_b)
                && expressions_equivalent(right_a, right_b)
        }

        // Unary operators - compare operator and operand
        (
            Expr::UnaryOp {
                op: op_a,
                expr: expr_a,
            },
            Expr::UnaryOp {
                op: op_b,
                expr: expr_b,
            },
        ) => op_a == op_b && expressions_equivalent(expr_a, expr_b),

        // Cast - compare expression and target type
        (
            Expr::Cast {
                expr: expr_a,
                target_type: type_a,
            },
            Expr::Cast {
                expr: expr_b,
                target_type: type_b,
            },
        ) => type_a == type_b && expressions_equivalent(expr_a, expr_b),

        // IsNull - compare inner expression
        (Expr::IsNull { expr: expr_a }, Expr::IsNull { expr: expr_b }) => {
            expressions_equivalent(expr_a, expr_b)
        }

        // IsNotNull - compare inner expression
        (Expr::IsNotNull { expr: expr_a }, Expr::IsNotNull { expr: expr_b }) => {
            expressions_equivalent(expr_a, expr_b)
        }

        // Between - compare all three expressions
        (
            Expr::Between {
                expr: expr_a,
                low: low_a,
                high: high_a,
            },
            Expr::Between {
                expr: expr_b,
                low: low_b,
                high: high_b,
            },
        ) => {
            expressions_equivalent(expr_a, expr_b)
                && expressions_equivalent(low_a, low_b)
                && expressions_equivalent(high_a, high_b)
        }

        // InList - compare expression, list, and negated flag
        (
            Expr::InList {
                expr: expr_a,
                list: list_a,
                negated: neg_a,
            },
            Expr::InList {
                expr: expr_b,
                list: list_b,
                negated: neg_b,
            },
        ) => {
            neg_a == neg_b
                && expressions_equivalent(expr_a, expr_b)
                && list_a.len() == list_b.len()
                && list_a
                    .iter()
                    .zip(list_b.iter())
                    .all(|(item_a, item_b)| expressions_equivalent(item_a, item_b))
        }

        // Like - compare expression, pattern, and negated flag
        (
            Expr::Like {
                expr: expr_a,
                pattern: pat_a,
                negated: neg_a,
            },
            Expr::Like {
                expr: expr_b,
                pattern: pat_b,
                negated: neg_b,
            },
        ) => {
            neg_a == neg_b
                && expressions_equivalent(expr_a, expr_b)
                && expressions_equivalent(pat_a, pat_b)
        }

        // JsonExtract - compare object and key
        (
            Expr::JsonExtract {
                object: obj_a,
                key: key_a,
            },
            Expr::JsonExtract {
                object: obj_b,
                key: key_b,
            },
        ) => expressions_equivalent(obj_a, obj_b) && expressions_equivalent(key_a, key_b),

        // JsonExtractText - compare object and key
        (
            Expr::JsonExtractText {
                object: obj_a,
                key: key_a,
            },
            Expr::JsonExtractText {
                object: obj_b,
                key: key_b,
            },
        ) => expressions_equivalent(obj_a, obj_b) && expressions_equivalent(key_a, key_b),

        // JsonContains - compare object and pattern
        (
            Expr::JsonContains {
                object: obj_a,
                pattern: pat_a,
            },
            Expr::JsonContains {
                object: obj_b,
                pattern: pat_b,
            },
        ) => expressions_equivalent(obj_a, obj_b) && expressions_equivalent(pat_a, pat_b),

        // Different expression types - not equivalent
        _ => false,
    }
}

/// Compare two literals for equivalence
fn literals_equivalent(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Null, Literal::Null) => true,
        (Literal::Boolean(a), Literal::Boolean(b)) => a == b,
        (Literal::Int(a), Literal::Int(b)) => a == b,
        (Literal::BigInt(a), Literal::BigInt(b)) => a == b,
        (Literal::Double(a), Literal::Double(b)) => {
            // Handle NaN and floating point comparison
            if a.is_nan() && b.is_nan() {
                true
            } else {
                (a - b).abs() < f64::EPSILON
            }
        }
        (Literal::Text(a), Literal::Text(b)) => a == b,
        (Literal::Uuid(a), Literal::Uuid(b)) => a == b,
        (Literal::Path(a), Literal::Path(b)) => a == b,
        (Literal::JsonB(a), Literal::JsonB(b)) => a == b,
        (Literal::Vector(a), Literal::Vector(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| (x - y).abs() < f32::EPSILON)
        }
        // Different literal types - not equivalent
        _ => false,
    }
}
