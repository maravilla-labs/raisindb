//! Literal comparison functions for sort operations
//!
//! Provides ordering comparisons between SQL literal values with proper NULL
//! handling and cross-type numeric comparisons.

use raisin_error::Error;
use raisin_sql::analyzer::Literal;
use std::cmp::Ordering;

/// Compare two rows based on pre-evaluated sort expression values
///
/// Used during sorting after all sort expressions have been pre-evaluated.
/// Compares pre-computed values instead of re-evaluating expressions.
pub(super) fn compare_evaluated_rows(
    a_values: &[Literal],
    b_values: &[Literal],
    sort_exprs: &[raisin_sql::logical_plan::SortExpr],
) -> Ordering {
    for (i, sort_expr) in sort_exprs.iter().enumerate() {
        let a_val = &a_values[i];
        let b_val = &b_values[i];

        let cmp = match compare_literals_with_nulls(a_val, b_val, sort_expr.nulls_first) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if cmp != Ordering::Equal {
            return if sort_expr.ascending {
                cmp
            } else {
                cmp.reverse()
            };
        }
    }

    Ordering::Equal
}

/// Compare two rows based on sort expressions (legacy, kept for compatibility)
#[allow(dead_code)]
pub(super) fn compare_rows(
    a: &super::Row,
    b: &super::Row,
    sort_exprs: &[raisin_sql::logical_plan::SortExpr],
) -> Ordering {
    use super::eval_expr;

    for sort_expr in sort_exprs {
        let a_val = eval_expr(&sort_expr.expr, a);
        let b_val = eval_expr(&sort_expr.expr, b);

        let (a_val, b_val) = match (a_val, b_val) {
            (Ok(a), Ok(b)) => (a, b),
            _ => continue,
        };

        let cmp = match compare_literals(&a_val, &b_val) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if cmp != Ordering::Equal {
            return if sort_expr.ascending {
                cmp
            } else {
                cmp.reverse()
            };
        }
    }

    Ordering::Equal
}

/// Helper to compare two vectors of literals
pub(super) fn compare_literals_vec(a: &[Literal], b: &[Literal]) -> Ordering {
    for (a_val, b_val) in a.iter().zip(b.iter()) {
        match compare_literals(a_val, b_val) {
            Ok(Ordering::Equal) => continue,
            Ok(ord) => return ord,
            Err(_) => continue,
        }
    }
    Ordering::Equal
}

/// Compare two literals with NULL handling based on nulls_first setting
pub(super) fn compare_literals_with_nulls(
    a: &Literal,
    b: &Literal,
    nulls_first: bool,
) -> Result<Ordering, Error> {
    match (a, b) {
        (Literal::Null, Literal::Null) => Ok(Ordering::Equal),
        (Literal::Null, _) => Ok(if nulls_first {
            Ordering::Less
        } else {
            Ordering::Greater
        }),
        (_, Literal::Null) => Ok(if nulls_first {
            Ordering::Greater
        } else {
            Ordering::Less
        }),
        (Literal::Boolean(a), Literal::Boolean(b)) => Ok(a.cmp(b)),
        (Literal::Int(a), Literal::Int(b)) => Ok(a.cmp(b)),
        (Literal::BigInt(a), Literal::BigInt(b)) => Ok(a.cmp(b)),
        (Literal::Double(a), Literal::Double(b)) => {
            if a < b {
                Ok(Ordering::Less)
            } else if a > b {
                Ok(Ordering::Greater)
            } else {
                Ok(Ordering::Equal)
            }
        }
        (
            a @ (Literal::Int(_) | Literal::BigInt(_) | Literal::Double(_)),
            b @ (Literal::Int(_) | Literal::BigInt(_) | Literal::Double(_)),
        ) => {
            let a_f = literal_to_f64(a)?;
            let b_f = literal_to_f64(b)?;
            if a_f < b_f {
                Ok(Ordering::Less)
            } else if a_f > b_f {
                Ok(Ordering::Greater)
            } else {
                Ok(Ordering::Equal)
            }
        }
        (Literal::Text(a), Literal::Text(b)) => Ok(a.cmp(b)),
        (Literal::Path(a), Literal::Path(b)) => Ok(a.cmp(b)),
        (Literal::Uuid(a), Literal::Uuid(b)) => Ok(a.cmp(b)),
        (Literal::Timestamp(a), Literal::Timestamp(b)) => Ok(a.cmp(b)),
        (Literal::JsonB(a), Literal::JsonB(b)) => {
            let a_str = a.to_string();
            let b_str = b.to_string();
            Ok(a_str.cmp(&b_str))
        }
        _ => Err(Error::Validation(format!(
            "Cannot compare {:?} and {:?}",
            a, b
        ))),
    }
}

/// Compare two literals (defaults to NULLS FIRST for backward compatibility)
pub(super) fn compare_literals(a: &Literal, b: &Literal) -> Result<Ordering, Error> {
    compare_literals_with_nulls(a, b, true)
}

/// Convert a literal to f64 for numeric comparison
fn literal_to_f64(lit: &Literal) -> Result<f64, Error> {
    match lit {
        Literal::Int(i) => Ok(*i as f64),
        Literal::BigInt(i) => Ok(*i as f64),
        Literal::Double(f) => Ok(*f),
        _ => Err(Error::Validation("Cannot convert to number".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_literals_numeric() {
        let a = Literal::Int(5);
        let b = Literal::Int(10);
        assert_eq!(compare_literals(&a, &b).unwrap(), Ordering::Less);
        assert_eq!(compare_literals(&b, &a).unwrap(), Ordering::Greater);
        assert_eq!(compare_literals(&a, &a).unwrap(), Ordering::Equal);
    }

    #[test]
    fn test_compare_literals_text() {
        let a = Literal::Text("apple".to_string());
        let b = Literal::Text("banana".to_string());
        assert_eq!(compare_literals(&a, &b).unwrap(), Ordering::Less);
        assert_eq!(compare_literals(&b, &a).unwrap(), Ordering::Greater);
        assert_eq!(compare_literals(&a, &a).unwrap(), Ordering::Equal);
    }

    #[test]
    fn test_compare_literals_null() {
        let null = Literal::Null;
        let value = Literal::Int(42);
        assert_eq!(compare_literals(&null, &value).unwrap(), Ordering::Less);
        assert_eq!(compare_literals(&value, &null).unwrap(), Ordering::Greater);
        assert_eq!(compare_literals(&null, &null).unwrap(), Ordering::Equal);
    }

    #[test]
    fn test_compare_literals_nulls_first() {
        let null = Literal::Null;
        let value = Literal::Int(42);
        assert_eq!(
            compare_literals_with_nulls(&null, &value, true).unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_literals_with_nulls(&value, &null, true).unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn test_compare_literals_nulls_last() {
        let null = Literal::Null;
        let value = Literal::Int(42);
        assert_eq!(
            compare_literals_with_nulls(&null, &value, false).unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_literals_with_nulls(&value, &null, false).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn test_compare_literals_null_equal() {
        let null = Literal::Null;
        assert_eq!(
            compare_literals_with_nulls(&null, &null, true).unwrap(),
            Ordering::Equal
        );
        assert_eq!(
            compare_literals_with_nulls(&null, &null, false).unwrap(),
            Ordering::Equal
        );
    }
}
