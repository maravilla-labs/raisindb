//! Predicate extraction utilities
//!
//! This module contains helper functions for extracting special predicates
//! from WHERE clauses, including revision, branch, and locale filters.

use super::super::{
    typed_expr::{BinaryOperator, Expr, Literal, TypedExpr},
    types::DataType,
};

/// Extract revision predicate from WHERE clause
///
/// Searches for `__revision = N` and removes it from the filter.
/// Returns (max_revision, remaining_filter).
///
/// # Examples
/// - `__revision = 342` → Some(342), None
/// - `__revision IS NULL` → None (HEAD), None
/// - `__revision = 342 AND other` → Some(342), Some(other)
/// - `other AND __revision = 342` → Some(342), Some(other)
/// - `other` → None (HEAD), Some(other)
pub(super) fn extract_revision_predicate(
    filter: &TypedExpr,
) -> (Option<raisin_hlc::HLC>, Option<TypedExpr>) {
    match &filter.expr {
        // Case 1: Direct __revision comparison
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Check if left side is __revision column
            if let Expr::Column { column, .. } = &left.expr {
                if column == "__revision" {
                    // Extract revision from right side
                    if let Some(rev) = extract_revision_value(right) {
                        return (rev, None);
                    }
                }
            }
            // Check if right side is __revision column
            if let Expr::Column { column, .. } = &right.expr {
                if column == "__revision" {
                    // Extract revision from left side
                    if let Some(rev) = extract_revision_value(left) {
                        return (rev, None);
                    }
                }
            }
            // Not a __revision predicate, keep the filter
            (None, Some(filter.clone()))
        }
        // Case 2: __revision IS NULL (treat as HEAD)
        Expr::IsNull { expr } => {
            if let Expr::Column { column, .. } = &expr.expr {
                if column == "__revision" {
                    return (None, None);
                }
            }
            (None, Some(filter.clone()))
        }
        // Case 3: AND expression - recursively extract from both sides
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let (left_rev, left_remaining) = extract_revision_predicate(left);
            let (right_rev, right_remaining) = extract_revision_predicate(right);

            // Prefer non-None revision value
            let revision = left_rev.or(right_rev);

            // Reconstruct remaining filter
            let remaining = match (left_remaining, right_remaining) {
                (Some(l), Some(r)) => {
                    // Both sides have remaining predicates, reconstruct AND
                    Some(TypedExpr::new(
                        Expr::BinaryOp {
                            left: Box::new(l),
                            op: BinaryOperator::And,
                            right: Box::new(r),
                        },
                        DataType::Boolean,
                    ))
                }
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            };

            (revision, remaining)
        }
        // Default: not a __revision predicate
        _ => (None, Some(filter.clone())),
    }
}

/// Extract revision value from a typed expression
/// Returns Some(revision) for numeric literals, None for IS NULL
fn extract_revision_value(expr: &TypedExpr) -> Option<Option<raisin_hlc::HLC>> {
    match &expr.expr {
        Expr::Literal(Literal::Int(i)) if *i >= 0 => Some(Some(raisin_hlc::HLC::new(*i as u64, 0))),
        Expr::Literal(Literal::BigInt(i)) if *i >= 0 => {
            Some(Some(raisin_hlc::HLC::new(*i as u64, 0)))
        }
        Expr::Literal(Literal::Null) => Some(None),
        _ => None,
    }
}

/// Extract branch predicate from WHERE clause
///
/// Searches for `__branch = 'branch_name'` and removes it from the filter.
/// Returns (branch_override, remaining_filter).
///
/// # Examples
/// - `WHERE __branch = 'staging'` → (Some("staging"), None)
/// - `WHERE __branch = 'dev' AND path = '/content'` → (Some("dev"), Some(path = '/content'))
/// - `WHERE path = '/content'` → (None, Some(path = '/content'))
pub(super) fn extract_branch_predicate(filter: &TypedExpr) -> (Option<String>, Option<TypedExpr>) {
    match &filter.expr {
        // Case 1: Direct __branch comparison
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Check if left side is __branch column
            if let Expr::Column { column, .. } = &left.expr {
                if column == "__branch" {
                    // Extract branch name from right side
                    if let Some(branch) = extract_branch_value(right) {
                        return (branch, None);
                    }
                }
            }
            // Check if right side is __branch column
            if let Expr::Column { column, .. } = &right.expr {
                if column == "__branch" {
                    // Extract branch name from left side
                    if let Some(branch) = extract_branch_value(left) {
                        return (branch, None);
                    }
                }
            }
            // Not a __branch predicate, keep the filter
            (None, Some(filter.clone()))
        }
        // Case 2: AND expression - recursively extract from both sides
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let (left_branch, left_remaining) = extract_branch_predicate(left);
            let (right_branch, right_remaining) = extract_branch_predicate(right);

            // Prefer non-None branch value (first match wins)
            let branch = left_branch.or(right_branch);

            // Reconstruct remaining filter
            let remaining = match (left_remaining, right_remaining) {
                (Some(l), Some(r)) => {
                    // Both sides have remaining predicates, reconstruct AND
                    Some(TypedExpr::new(
                        Expr::BinaryOp {
                            left: Box::new(l),
                            op: BinaryOperator::And,
                            right: Box::new(r),
                        },
                        DataType::Boolean,
                    ))
                }
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            };

            (branch, remaining)
        }
        // Default: not a __branch predicate
        _ => (None, Some(filter.clone())),
    }
}

/// Extract branch name from a typed expression
/// Returns Some(branch_name) for string literals
fn extract_branch_value(expr: &TypedExpr) -> Option<Option<String>> {
    match &expr.expr {
        Expr::Literal(Literal::Text(s)) => Some(Some(s.clone())),
        _ => None,
    }
}

/// Extract locale predicate from WHERE clause
///
/// Searches for `locale = 'en'` or `locale IN ('en', 'de')` and removes it from the filter.
/// Returns (locales, remaining_filter).
///
/// # Examples
/// - `WHERE locale = 'en'` → (vec!["en"], None)
/// - `WHERE locale IN ('en', 'de')` → (vec!["en", "de"], None)
/// - `WHERE locale = 'en' AND path = '/content'` → (vec!["en"], Some(path = '/content'))
/// - `WHERE path = '/content'` → (vec![], Some(path = '/content'))
pub(super) fn extract_locale_predicate(filter: &TypedExpr) -> (Vec<String>, Option<TypedExpr>) {
    match &filter.expr {
        // Case 1: Direct locale comparison (locale = 'en')
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Check if left side is locale column
            if let Expr::Column { column, .. } = &left.expr {
                if column == "locale" {
                    // Extract locale value from right side
                    if let Some(locale) = extract_locale_value(right) {
                        return (vec![locale], None);
                    }
                }
            }
            // Check if right side is locale column
            if let Expr::Column { column, .. } = &right.expr {
                if column == "locale" {
                    // Extract locale value from left side
                    if let Some(locale) = extract_locale_value(left) {
                        return (vec![locale], None);
                    }
                }
            }
            // Not a locale predicate, keep the filter
            (vec![], Some(filter.clone()))
        }
        // Case 2: IN list (locale IN ('en', 'de'))
        Expr::InList {
            expr,
            list,
            negated,
        } if !negated => {
            if let Expr::Column { column, .. } = &expr.expr {
                if column == "locale" {
                    // Extract all locale values from the list
                    let mut locales = Vec::new();
                    for item in list {
                        if let Some(locale) = extract_locale_value(item) {
                            locales.push(locale);
                        }
                    }
                    if !locales.is_empty() {
                        return (locales, None);
                    }
                }
            }
            // Not a locale predicate, keep the filter
            (vec![], Some(filter.clone()))
        }
        // Case 3: AND expression - recursively extract from both sides
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let (left_locales, left_remaining) = extract_locale_predicate(left);
            let (right_locales, right_remaining) = extract_locale_predicate(right);

            // Combine locales (left takes precedence if both have locales)
            let locales = if !left_locales.is_empty() {
                left_locales
            } else {
                right_locales
            };

            // Reconstruct remaining filter
            let remaining = match (left_remaining, right_remaining) {
                (Some(l), Some(r)) => {
                    // Both sides have remaining predicates, reconstruct AND
                    Some(TypedExpr::new(
                        Expr::BinaryOp {
                            left: Box::new(l),
                            op: BinaryOperator::And,
                            right: Box::new(r),
                        },
                        DataType::Boolean,
                    ))
                }
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            };

            (locales, remaining)
        }
        // Default: not a locale predicate
        _ => (vec![], Some(filter.clone())),
    }
}

/// Extract locale value from a typed expression
/// Returns Some(locale_code) for string literals
fn extract_locale_value(expr: &TypedExpr) -> Option<String> {
    match &expr.expr {
        Expr::Literal(Literal::Text(s)) => Some(s.clone()),
        _ => None,
    }
}
