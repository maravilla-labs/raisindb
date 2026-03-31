//! Binary and unary operator evaluation, value comparison, and arithmetic.

use raisin_sql::ast::{BinaryOperator, UnaryOperator};

use super::Result;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::types::SqlValue;

/// Evaluate binary operator
pub(super) fn evaluate_binary_op(
    op: BinaryOperator,
    left: SqlValue,
    right: SqlValue,
) -> Result<SqlValue> {
    match op {
        // Comparison operators
        BinaryOperator::Eq => Ok(SqlValue::Boolean(values_equal(&left, &right))),
        BinaryOperator::NotEq => Ok(SqlValue::Boolean(!values_equal(&left, &right))),
        BinaryOperator::Lt => Ok(SqlValue::Boolean(
            compare_values(&left, &right) == Some(std::cmp::Ordering::Less),
        )),
        BinaryOperator::LtEq => Ok(SqlValue::Boolean(
            compare_values(&left, &right).map(|o| o != std::cmp::Ordering::Greater) == Some(true),
        )),
        BinaryOperator::Gt => Ok(SqlValue::Boolean(
            compare_values(&left, &right) == Some(std::cmp::Ordering::Greater),
        )),
        BinaryOperator::GtEq => Ok(SqlValue::Boolean(
            compare_values(&left, &right).map(|o| o != std::cmp::Ordering::Less) == Some(true),
        )),

        // Logical operators
        BinaryOperator::And => {
            let l = left.as_bool().unwrap_or(false);
            let r = right.as_bool().unwrap_or(false);
            Ok(SqlValue::Boolean(l && r))
        }
        BinaryOperator::Or => {
            let l = left.as_bool().unwrap_or(false);
            let r = right.as_bool().unwrap_or(false);
            Ok(SqlValue::Boolean(l || r))
        }

        // Arithmetic operators
        BinaryOperator::Plus => arithmetic_op(left, right, |a, b| a + b, |a, b| a + b),
        BinaryOperator::Minus => arithmetic_op(left, right, |a, b| a - b, |a, b| a - b),
        BinaryOperator::Multiply => arithmetic_op(left, right, |a, b| a * b, |a, b| a * b),
        BinaryOperator::Divide => {
            // Handle division by zero
            if matches!(&right, SqlValue::Integer(0)) {
                return Err(ExecutionError::Validation("Division by zero".into()));
            }
            if matches!(&right, SqlValue::Float(f) if *f == 0.0) {
                return Err(ExecutionError::Validation("Division by zero".into()));
            }
            arithmetic_op(left, right, |a, b| a / b, |a, b| a / b)
        }
        BinaryOperator::Modulo => {
            if let (SqlValue::Integer(a), SqlValue::Integer(b)) = (&left, &right) {
                if *b == 0 {
                    return Err(ExecutionError::Validation("Modulo by zero".into()));
                }
                Ok(SqlValue::Integer(a % b))
            } else {
                Ok(SqlValue::Null)
            }
        }

        // String concatenation
        BinaryOperator::Concat => {
            let l = match &left {
                SqlValue::String(s) => s.clone(),
                SqlValue::Integer(i) => i.to_string(),
                SqlValue::Float(f) => f.to_string(),
                SqlValue::Boolean(b) => b.to_string(),
                _ => return Ok(SqlValue::Null),
            };
            let r = match &right {
                SqlValue::String(s) => s.clone(),
                SqlValue::Integer(i) => i.to_string(),
                SqlValue::Float(f) => f.to_string(),
                SqlValue::Boolean(b) => b.to_string(),
                _ => return Ok(SqlValue::Null),
            };
            Ok(SqlValue::String(format!("{}{}", l, r)))
        }
    }
}

/// Perform arithmetic operation
fn arithmetic_op<FI, FF>(
    left: SqlValue,
    right: SqlValue,
    int_op: FI,
    float_op: FF,
) -> Result<SqlValue>
where
    FI: Fn(i64, i64) -> i64,
    FF: Fn(f64, f64) -> f64,
{
    match (&left, &right) {
        (SqlValue::Integer(a), SqlValue::Integer(b)) => Ok(SqlValue::Integer(int_op(*a, *b))),
        (SqlValue::Float(a), SqlValue::Float(b)) => Ok(SqlValue::Float(float_op(*a, *b))),
        (SqlValue::Integer(a), SqlValue::Float(b)) => Ok(SqlValue::Float(float_op(*a as f64, *b))),
        (SqlValue::Float(a), SqlValue::Integer(b)) => Ok(SqlValue::Float(float_op(*a, *b as f64))),
        _ => Ok(SqlValue::Null),
    }
}

/// Evaluate unary operator
pub(super) fn evaluate_unary_op(op: UnaryOperator, val: SqlValue) -> Result<SqlValue> {
    match op {
        UnaryOperator::Not => {
            let b = val.as_bool().unwrap_or(false);
            Ok(SqlValue::Boolean(!b))
        }
        UnaryOperator::Minus => match val {
            SqlValue::Integer(i) => Ok(SqlValue::Integer(-i)),
            SqlValue::Float(f) => Ok(SqlValue::Float(-f)),
            _ => Ok(SqlValue::Null),
        },
        UnaryOperator::Plus => Ok(val),
    }
}

/// Check if two values are equal
pub(super) fn values_equal(left: &SqlValue, right: &SqlValue) -> bool {
    match (left, right) {
        (SqlValue::Null, SqlValue::Null) => true,
        (SqlValue::Null, _) | (_, SqlValue::Null) => false,
        (SqlValue::Boolean(a), SqlValue::Boolean(b)) => a == b,
        (SqlValue::Integer(a), SqlValue::Integer(b)) => a == b,
        (SqlValue::Float(a), SqlValue::Float(b)) => (a - b).abs() < f64::EPSILON,
        (SqlValue::Integer(a), SqlValue::Float(b)) => (*a as f64 - b).abs() < f64::EPSILON,
        (SqlValue::Float(a), SqlValue::Integer(b)) => (a - *b as f64).abs() < f64::EPSILON,
        (SqlValue::String(a), SqlValue::String(b)) => a == b,
        _ => false,
    }
}

/// Compare two values
pub(super) fn compare_values(left: &SqlValue, right: &SqlValue) -> Option<std::cmp::Ordering> {
    match (left, right) {
        (SqlValue::Integer(a), SqlValue::Integer(b)) => Some(a.cmp(b)),
        (SqlValue::Float(a), SqlValue::Float(b)) => a.partial_cmp(b),
        (SqlValue::Integer(a), SqlValue::Float(b)) => (*a as f64).partial_cmp(b),
        (SqlValue::Float(a), SqlValue::Integer(b)) => a.partial_cmp(&(*b as f64)),
        (SqlValue::String(a), SqlValue::String(b)) => Some(a.cmp(b)),
        _ => None,
    }
}
