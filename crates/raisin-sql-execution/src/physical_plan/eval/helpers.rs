//! Helper functions for expression evaluation
//!
//! This module contains utility functions used throughout expression evaluation,
//! including arithmetic operations, comparisons, and type conversions.

use raisin_error::Error;
use raisin_sql::analyzer::{BinaryOperator, Literal};

use super::casting::parse_timestamp;

/// Perform arithmetic operation on two literals
///
/// This function handles NULL propagation according to SQL semantics:
/// NULL in any arithmetic operation results in NULL.
#[inline]
pub(super) fn arithmetic_op<F>(left: &Literal, right: &Literal, op: F) -> Result<Literal, Error>
where
    F: Fn(f64, f64) -> f64,
{
    // SQL standard: NULL in arithmetic propagates to NULL result
    // NULL + x = NULL, x * NULL = NULL, NULL / x = NULL, x / NULL = NULL
    if matches!(left, Literal::Null) || matches!(right, Literal::Null) {
        return Ok(Literal::Null);
    }

    let left_num = literal_to_number(left)?;
    let right_num = literal_to_number(right)?;
    Ok(Literal::Double(op(left_num, right_num)))
}

/// Convert literal to number for arithmetic
#[inline]
pub(super) fn literal_to_number(lit: &Literal) -> Result<f64, Error> {
    match lit {
        Literal::Int(i) => Ok(*i as f64),
        Literal::BigInt(i) => Ok(*i as f64),
        Literal::Double(f) => Ok(*f),
        _ => Err(Error::Validation(format!(
            "Cannot convert {:?} to number",
            lit
        ))),
    }
}

/// Check if literal is zero
#[inline]
pub(super) fn is_zero(lit: &Literal) -> bool {
    match lit {
        Literal::Int(i) => *i == 0,
        Literal::BigInt(i) => *i == 0,
        Literal::Double(f) => *f == 0.0,
        _ => false,
    }
}

/// Compare two literals for equality
#[inline]
pub(super) fn literals_equal(left: &Literal, right: &Literal) -> Result<bool, Error> {
    match (left, right) {
        (Literal::Null, Literal::Null) => Ok(true),
        (Literal::Null, _) | (_, Literal::Null) => Ok(false),
        (Literal::Boolean(a), Literal::Boolean(b)) => Ok(a == b),
        (Literal::Int(a), Literal::Int(b)) => Ok(a == b),
        (Literal::BigInt(a), Literal::BigInt(b)) => Ok(a == b),
        (Literal::Double(a), Literal::Double(b)) => Ok((a - b).abs() < f64::EPSILON),
        (Literal::Text(a), Literal::Text(b)) => Ok(a == b),
        (Literal::Uuid(a), Literal::Uuid(b)) => Ok(a == b),
        // Cross-type Uuid comparisons (Uuid can compare with Text)
        (Literal::Uuid(a), Literal::Text(b)) | (Literal::Text(a), Literal::Uuid(b)) => Ok(a == b),
        (Literal::Path(a), Literal::Path(b)) => Ok(a == b),
        // Cross-type Path comparisons (Path can compare with Text)
        (Literal::Path(a), Literal::Text(b)) | (Literal::Text(a), Literal::Path(b)) => Ok(a == b),
        (Literal::JsonB(a), Literal::JsonB(b)) => Ok(a == b),
        // Timestamp comparisons
        (Literal::Timestamp(a), Literal::Timestamp(b)) => Ok(a == b),
        // Cross-type Timestamp comparisons (Timestamp can compare with Text)
        (Literal::Timestamp(ts), Literal::Text(s)) | (Literal::Text(s), Literal::Timestamp(ts)) => {
            match parse_timestamp(s) {
                Some(parsed) => Ok(ts == &parsed),
                None => Ok(false), // Unparseable text is not equal to any timestamp
            }
        }
        // Cross-type numeric comparisons
        (
            a @ (Literal::Int(_) | Literal::BigInt(_) | Literal::Double(_)),
            b @ (Literal::Int(_) | Literal::BigInt(_) | Literal::Double(_)),
        ) => {
            let a_num = literal_to_number(a)?;
            let b_num = literal_to_number(b)?;
            Ok((a_num - b_num).abs() < f64::EPSILON)
        }
        _ => Err(Error::Validation(format!(
            "Cannot compare {:?} and {:?}",
            left, right
        ))),
    }
}

/// Compare two literals with an operator
#[inline]
pub(super) fn compare_literals(
    left: &Literal,
    right: &Literal,
    op: BinaryOperator,
) -> Result<bool, Error> {
    // Handle NULL comparisons: NULL compared to anything (including NULL) is FALSE
    // This follows SQL semantics where NULL comparisons are neither true nor false,
    // but in boolean context (WHERE clause) they're treated as FALSE
    if matches!(left, Literal::Null) || matches!(right, Literal::Null) {
        return Ok(false);
    }

    // Handle numeric comparisons
    if matches!(
        left,
        Literal::Int(_) | Literal::BigInt(_) | Literal::Double(_)
    ) && matches!(
        right,
        Literal::Int(_) | Literal::BigInt(_) | Literal::Double(_)
    ) {
        let left_num = literal_to_number(left)?;
        let right_num = literal_to_number(right)?;

        return Ok(match op {
            BinaryOperator::Lt => left_num < right_num,
            BinaryOperator::LtEq => left_num <= right_num,
            BinaryOperator::Gt => left_num > right_num,
            BinaryOperator::GtEq => left_num >= right_num,
            _ => return Err(Error::Validation("Invalid comparison operator".to_string())),
        });
    }

    // Handle text comparisons
    if let (Literal::Text(a), Literal::Text(b)) = (left, right) {
        return Ok(match op {
            BinaryOperator::Lt => a < b,
            BinaryOperator::LtEq => a <= b,
            BinaryOperator::Gt => a > b,
            BinaryOperator::GtEq => a >= b,
            _ => return Err(Error::Validation("Invalid comparison operator".to_string())),
        });
    }

    // Handle path comparisons (Path can compare with Path or Text)
    match (left, right) {
        (Literal::Path(a), Literal::Path(b))
        | (Literal::Path(a), Literal::Text(b))
        | (Literal::Text(a), Literal::Path(b)) => {
            return Ok(match op {
                BinaryOperator::Lt => a < b,
                BinaryOperator::LtEq => a <= b,
                BinaryOperator::Gt => a > b,
                BinaryOperator::GtEq => a >= b,
                _ => return Err(Error::Validation("Invalid comparison operator".to_string())),
            });
        }
        _ => {}
    }

    // Handle timestamp comparisons
    if let (Literal::Timestamp(a), Literal::Timestamp(b)) = (left, right) {
        return Ok(match op {
            BinaryOperator::Lt => a < b,
            BinaryOperator::LtEq => a <= b,
            BinaryOperator::Gt => a > b,
            BinaryOperator::GtEq => a >= b,
            _ => return Err(Error::Validation("Invalid comparison operator".to_string())),
        });
    }

    // Handle cross-type timestamp comparisons (Timestamp vs Text)
    // This enables cursor-based pagination with text literals:
    // WHERE created_at < '2026-01-04T20:35:29.900186+00:00'
    match (left, right) {
        (Literal::Timestamp(ts), Literal::Text(s)) => {
            let parsed = parse_timestamp(s).ok_or_else(|| {
                Error::Validation(format!("Cannot parse '{}' as timestamp for comparison", s))
            })?;
            return Ok(match op {
                BinaryOperator::Lt => ts < &parsed,
                BinaryOperator::LtEq => ts <= &parsed,
                BinaryOperator::Gt => ts > &parsed,
                BinaryOperator::GtEq => ts >= &parsed,
                _ => return Err(Error::Validation("Invalid comparison operator".to_string())),
            });
        }
        (Literal::Text(s), Literal::Timestamp(ts)) => {
            let parsed = parse_timestamp(s).ok_or_else(|| {
                Error::Validation(format!("Cannot parse '{}' as timestamp for comparison", s))
            })?;
            return Ok(match op {
                BinaryOperator::Lt => &parsed < ts,
                BinaryOperator::LtEq => &parsed <= ts,
                BinaryOperator::Gt => &parsed > ts,
                BinaryOperator::GtEq => &parsed >= ts,
                _ => return Err(Error::Validation("Invalid comparison operator".to_string())),
            });
        }
        _ => {}
    }

    Err(Error::Validation(format!(
        "Cannot compare {:?} and {:?}",
        left, right
    )))
}

/// Logical AND operation
pub(super) fn logical_and(left: &Literal, right: &Literal) -> Result<Literal, Error> {
    match (left, right) {
        (Literal::Boolean(a), Literal::Boolean(b)) => Ok(Literal::Boolean(*a && *b)),
        _ => Err(Error::Validation(
            "AND requires boolean operands".to_string(),
        )),
    }
}

/// Logical OR operation
pub(super) fn logical_or(left: &Literal, right: &Literal) -> Result<Literal, Error> {
    match (left, right) {
        (Literal::Boolean(a), Literal::Boolean(b)) => Ok(Literal::Boolean(*a || *b)),
        _ => Err(Error::Validation(
            "OR requires boolean operands".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn make_timestamp(s: &str) -> Literal {
        let dt = DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        Literal::Timestamp(dt)
    }

    #[test]
    fn test_compare_timestamp_with_text_lt() {
        // Simulate: created_at < '2026-01-04T20:35:29.900186+00:00'
        // where created_at is 2026-01-04T20:00:00Z (earlier)
        let ts = make_timestamp("2026-01-04T20:00:00Z");
        let text = Literal::Text("2026-01-04T20:35:29.900186+00:00".to_string());

        let result = compare_literals(&ts, &text, BinaryOperator::Lt).unwrap();
        assert!(
            result,
            "Earlier timestamp should be less than later timestamp"
        );
    }

    #[test]
    fn test_compare_timestamp_with_text_gt() {
        // Simulate: created_at > '2026-01-04T20:00:00Z'
        // where created_at is 2026-01-04T20:35:29.900186+00:00 (later)
        let ts = make_timestamp("2026-01-04T20:35:29.900186+00:00");
        let text = Literal::Text("2026-01-04T20:00:00Z".to_string());

        let result = compare_literals(&ts, &text, BinaryOperator::Gt).unwrap();
        assert!(
            result,
            "Later timestamp should be greater than earlier timestamp"
        );
    }

    #[test]
    fn test_compare_text_with_timestamp_lt() {
        // Simulate: '2026-01-04T20:00:00Z' < created_at
        // where created_at is 2026-01-04T20:35:29.900186+00:00 (later)
        let text = Literal::Text("2026-01-04T20:00:00Z".to_string());
        let ts = make_timestamp("2026-01-04T20:35:29.900186+00:00");

        let result = compare_literals(&text, &ts, BinaryOperator::Lt).unwrap();
        assert!(
            result,
            "Earlier text timestamp should be less than later timestamp"
        );
    }

    #[test]
    fn test_compare_timestamp_with_text_eq() {
        // Equal timestamps
        let ts = make_timestamp("2026-01-04T20:35:29.900186+00:00");
        let text = Literal::Text("2026-01-04T20:35:29.900186+00:00".to_string());

        let result = literals_equal(&ts, &text).unwrap();
        assert!(result, "Equal timestamps should be equal");
    }

    #[test]
    fn test_compare_timestamp_with_text_not_eq() {
        // Not equal timestamps
        let ts = make_timestamp("2026-01-04T20:00:00Z");
        let text = Literal::Text("2026-01-04T20:35:29.900186+00:00".to_string());

        let result = literals_equal(&ts, &text).unwrap();
        assert!(!result, "Different timestamps should not be equal");
    }

    #[test]
    fn test_compare_timestamp_with_invalid_text_eq() {
        // Invalid text should not be equal to any timestamp
        let ts = make_timestamp("2026-01-04T20:00:00Z");
        let text = Literal::Text("not a timestamp".to_string());

        let result = literals_equal(&ts, &text).unwrap();
        assert!(!result, "Invalid text should not equal any timestamp");
    }

    #[test]
    fn test_compare_timestamp_with_invalid_text_lt_error() {
        // Invalid text should error on comparison
        let ts = make_timestamp("2026-01-04T20:00:00Z");
        let text = Literal::Text("not a timestamp".to_string());

        let result = compare_literals(&ts, &text, BinaryOperator::Lt);
        assert!(
            result.is_err(),
            "Should error when comparing timestamp with invalid text"
        );
    }

    #[test]
    fn test_cursor_pagination_scenario() {
        // This is the exact scenario from the bug report:
        // SELECT * FROM 'raisin:access_control'
        // WHERE created_at < '2026-01-04T20:35:29.900186+00:00'

        // Simulate multiple rows with different timestamps
        let cursor = Literal::Text("2026-01-04T20:35:29.900186+00:00".to_string());

        // Rows that should match (created_at < cursor)
        let row1 = make_timestamp("2026-01-04T10:00:00Z");
        let row2 = make_timestamp("2026-01-04T20:00:00Z");
        let row3 = make_timestamp("2026-01-04T20:35:29.900185+00:00"); // 1 microsecond earlier

        // Row that should NOT match (created_at >= cursor)
        let row4 = make_timestamp("2026-01-04T20:35:29.900186+00:00"); // Equal
        let row5 = make_timestamp("2026-01-05T00:00:00Z"); // Later

        // Test comparisons
        assert!(compare_literals(&row1, &cursor, BinaryOperator::Lt).unwrap());
        assert!(compare_literals(&row2, &cursor, BinaryOperator::Lt).unwrap());
        assert!(compare_literals(&row3, &cursor, BinaryOperator::Lt).unwrap());
        assert!(!compare_literals(&row4, &cursor, BinaryOperator::Lt).unwrap());
        assert!(!compare_literals(&row5, &cursor, BinaryOperator::Lt).unwrap());
    }
}
