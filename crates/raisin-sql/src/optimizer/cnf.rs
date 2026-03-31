//! Conjunctive Normal Form (CNF) Utilities
//!
//! Provides utilities for normalizing boolean expressions into CNF, which simplifies
//! predicate analysis and optimization.
//!
//! # CNF Definition
//!
//! A boolean expression is in CNF if it's an AND of ORs:
//! ```text
//! (a OR b) AND (c OR d) AND e
//! ```
//!
//! # Usage
//!
//! For query optimization, we primarily need to extract conjuncts (AND-ed terms)
//! from filter predicates. This allows us to:
//! - Analyze each predicate independently
//! - Push down individual predicates
//! - Reorder predicates by selectivity
//!
//! # Current Implementation
//!
//! Currently, we focus on flattening AND operations, which is the most common
//! case in SQL WHERE clauses. Full CNF conversion (including OR distribution)
//! is not yet implemented.

use crate::analyzer::{BinaryOperator, Expr, TypedExpr};

/// Flatten AND operations into a list of conjuncts
///
/// This recursively traverses a boolean expression and extracts all terms
/// that are AND-ed together.
///
/// # Examples
///
/// ```text
/// a AND b AND c        → [a, b, c]
/// (a AND b) AND c      → [a, b, c]
/// a AND (b AND c)      → [a, b, c]
/// a OR b               → [a OR b]  (not flattened)
/// a AND (b OR c)       → [a, (b OR c)]
/// ```
pub fn flatten_ands(expr: TypedExpr) -> Vec<TypedExpr> {
    match expr.expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut conjuncts = flatten_ands(*left);
            conjuncts.extend(flatten_ands(*right));
            conjuncts
        }
        _ => vec![expr],
    }
}

/// Collect all conjuncts from a filter predicate
///
/// This is an alias for `flatten_ands` with a more semantic name
/// for use in optimization passes.
pub fn collect_conjuncts(predicate: &TypedExpr) -> Vec<TypedExpr> {
    flatten_ands(predicate.clone())
}

/// Combine conjuncts back into a single AND expression
///
/// This is the inverse of `flatten_ands`. It takes a list of expressions
/// and combines them with AND operators.
///
/// # Examples
///
/// ```text
/// [a]           → a
/// [a, b]        → a AND b
/// [a, b, c]     → a AND b AND c
/// []            → None
/// ```
pub fn combine_conjuncts(conjuncts: Vec<TypedExpr>) -> Option<TypedExpr> {
    use crate::analyzer::DataType;

    if conjuncts.is_empty() {
        return None;
    }

    if conjuncts.len() == 1 {
        return Some(conjuncts.into_iter().next().expect("non-empty after length check"));
    }

    // Build nested AND tree (left-associative)
    let mut iter = conjuncts.into_iter();
    let mut result = iter.next().expect("non-empty after length check");

    for conjunct in iter {
        result = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(result),
                op: BinaryOperator::And,
                right: Box::new(conjunct),
            },
            DataType::Boolean,
        );
    }

    Some(result)
}

/// Check if an expression is in CNF
///
/// An expression is in CNF if it's an AND of clauses, where each clause
/// is either:
/// - A literal (column, constant, or negation)
/// - An OR of literals
///
/// Note: This is a simplified check that doesn't validate full CNF.
pub fn is_cnf(expr: &TypedExpr) -> bool {
    match &expr.expr {
        // Literals are trivially CNF
        Expr::Literal(_) | Expr::Column { .. } => true,

        // NOT is CNF if its operand is a literal
        Expr::UnaryOp { expr, .. } => matches!(expr.expr, Expr::Literal(_) | Expr::Column { .. }),

        // AND is CNF if both sides are CNF clauses
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => is_cnf_clause(left) && is_cnf_clause(right),

        // A single OR clause is CNF
        Expr::BinaryOp {
            op: BinaryOperator::Or,
            ..
        } => is_cnf_clause(expr),

        // Other expressions need checking
        _ => is_cnf_clause(expr),
    }
}

/// Check if an expression is a valid CNF clause
///
/// A CNF clause is either:
/// - A literal
/// - An OR of literals
fn is_cnf_clause(expr: &TypedExpr) -> bool {
    match &expr.expr {
        // Literals are valid clauses
        Expr::Literal(_) | Expr::Column { .. } => true,

        // NOT of literal is valid
        Expr::UnaryOp { expr, .. } => matches!(expr.expr, Expr::Literal(_) | Expr::Column { .. }),

        // OR is valid if both sides are literals
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Or,
            right,
        } => is_literal(left) && is_literal(right),

        // Comparisons and other predicates are treated as literals
        Expr::BinaryOp { .. } => true,
        Expr::Function { .. } => true,
        Expr::IsNull { .. } | Expr::IsNotNull { .. } => true,
        Expr::Between { .. } => true,
        Expr::InList { .. } => true,
        Expr::JsonExtractText { .. } | Expr::JsonContains { .. } | Expr::JsonKeyExists { .. } => {
            true
        }

        _ => false,
    }
}

/// Check if an expression is a literal (for CNF purposes)
fn is_literal(expr: &TypedExpr) -> bool {
    matches!(
        expr.expr,
        Expr::Literal(_)
            | Expr::Column { .. }
            | Expr::BinaryOp { .. }
            | Expr::Function { .. }
            | Expr::IsNull { .. }
            | Expr::IsNotNull { .. }
            | Expr::Between { .. }
            | Expr::InList { .. }
            | Expr::JsonExtractText { .. }
            | Expr::JsonContains { .. }
            | Expr::JsonKeyExists { .. }
    ) || matches!(expr.expr, Expr::UnaryOp { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{DataType, Literal};

    fn make_column(name: &str) -> TypedExpr {
        TypedExpr::column("nodes".to_string(), name.to_string(), DataType::Text)
    }

    fn make_literal(val: i32) -> TypedExpr {
        TypedExpr::literal(Literal::Int(val))
    }

    fn make_and(left: TypedExpr, right: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::And,
                right: Box::new(right),
            },
            DataType::Boolean,
        )
    }

    fn make_or(left: TypedExpr, right: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::Or,
                right: Box::new(right),
            },
            DataType::Boolean,
        )
    }

    #[test]
    fn test_flatten_ands_single() {
        let expr = make_column("a");
        let conjuncts = flatten_ands(expr);
        assert_eq!(conjuncts.len(), 1);
    }

    #[test]
    fn test_flatten_ands_simple() {
        // a AND b
        let expr = make_and(make_column("a"), make_column("b"));
        let conjuncts = flatten_ands(expr);
        assert_eq!(conjuncts.len(), 2);
    }

    #[test]
    fn test_flatten_ands_nested_left() {
        // (a AND b) AND c
        let left = make_and(make_column("a"), make_column("b"));
        let expr = make_and(left, make_column("c"));
        let conjuncts = flatten_ands(expr);
        assert_eq!(conjuncts.len(), 3);
    }

    #[test]
    fn test_flatten_ands_nested_right() {
        // a AND (b AND c)
        let right = make_and(make_column("b"), make_column("c"));
        let expr = make_and(make_column("a"), right);
        let conjuncts = flatten_ands(expr);
        assert_eq!(conjuncts.len(), 3);
    }

    #[test]
    fn test_flatten_ands_deeply_nested() {
        // ((a AND b) AND c) AND d
        let ab = make_and(make_column("a"), make_column("b"));
        let abc = make_and(ab, make_column("c"));
        let expr = make_and(abc, make_column("d"));
        let conjuncts = flatten_ands(expr);
        assert_eq!(conjuncts.len(), 4);
    }

    #[test]
    fn test_flatten_ands_with_or() {
        // a AND (b OR c)
        let or_expr = make_or(make_column("b"), make_column("c"));
        let expr = make_and(make_column("a"), or_expr);
        let conjuncts = flatten_ands(expr);

        // Should produce 2 conjuncts: [a, (b OR c)]
        assert_eq!(conjuncts.len(), 2);

        // First conjunct should be 'a'
        assert!(matches!(conjuncts[0].expr, Expr::Column { .. }));

        // Second conjunct should be (b OR c)
        assert!(matches!(
            conjuncts[1].expr,
            Expr::BinaryOp {
                op: BinaryOperator::Or,
                ..
            }
        ));
    }

    #[test]
    fn test_collect_conjuncts() {
        // Test that collect_conjuncts is an alias for flatten_ands
        let expr = make_and(make_column("a"), make_column("b"));
        let conjuncts = collect_conjuncts(&expr);
        assert_eq!(conjuncts.len(), 2);
    }

    #[test]
    fn test_combine_conjuncts_empty() {
        let result = combine_conjuncts(vec![]);
        assert!(result.is_none());
    }

    #[test]
    fn test_combine_conjuncts_single() {
        let expr = make_column("a");
        let result = combine_conjuncts(vec![expr.clone()]);
        assert!(result.is_some());

        // Should return the same expression
        let combined = result.unwrap();
        assert!(matches!(combined.expr, Expr::Column { .. }));
    }

    #[test]
    fn test_combine_conjuncts_two() {
        let a = make_column("a");
        let b = make_column("b");
        let result = combine_conjuncts(vec![a, b]);

        assert!(result.is_some());
        let combined = result.unwrap();

        // Should be a AND b
        assert!(matches!(
            combined.expr,
            Expr::BinaryOp {
                op: BinaryOperator::And,
                ..
            }
        ));
    }

    #[test]
    fn test_combine_conjuncts_three() {
        let a = make_column("a");
        let b = make_column("b");
        let c = make_column("c");
        let result = combine_conjuncts(vec![a, b, c]);

        assert!(result.is_some());
        let combined = result.unwrap();

        // Should be (a AND b) AND c
        assert!(matches!(
            combined.expr,
            Expr::BinaryOp {
                op: BinaryOperator::And,
                ..
            }
        ));
    }

    #[test]
    fn test_flatten_and_combine_roundtrip() {
        // Original: a AND b AND c
        let ab = make_and(make_column("a"), make_column("b"));
        let original = make_and(ab, make_column("c"));

        // Flatten
        let conjuncts = flatten_ands(original);
        assert_eq!(conjuncts.len(), 3);

        // Combine back
        let reconstructed = combine_conjuncts(conjuncts);
        assert!(reconstructed.is_some());

        // Should still be an AND expression
        let expr = reconstructed.unwrap();
        assert!(matches!(
            expr.expr,
            Expr::BinaryOp {
                op: BinaryOperator::And,
                ..
            }
        ));
    }

    #[test]
    fn test_is_cnf_literal() {
        assert!(is_cnf(&make_column("a")));
        assert!(is_cnf(&make_literal(42)));
    }

    #[test]
    fn test_is_cnf_simple_and() {
        let expr = make_and(make_column("a"), make_column("b"));
        assert!(is_cnf(&expr));
    }

    #[test]
    fn test_is_cnf_or_clause() {
        let expr = make_or(make_column("a"), make_column("b"));
        assert!(is_cnf(&expr));
    }

    #[test]
    fn test_is_cnf_and_of_ors() {
        // (a OR b) AND (c OR d)
        let left = make_or(make_column("a"), make_column("b"));
        let right = make_or(make_column("c"), make_column("d"));
        let expr = make_and(left, right);
        assert!(is_cnf(&expr));
    }
}
