//! Aggregate window functions
//!
//! Implements SUM, AVG, MIN, MAX, and COUNT window functions
//! that operate over window frames within partitions.

use super::compare::compare_literals;
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::{ExecutionError, Row};
use raisin_sql::analyzer::Literal;
use std::cmp::Ordering;

/// Compute SUM over a window frame
pub(crate) fn compute_sum_over_frame(
    result_rows: &[Row],
    frame_start: usize,
    frame_end: usize,
    arg_expr: &raisin_sql::analyzer::TypedExpr,
) -> Result<Literal, ExecutionError> {
    let mut sum = 0.0;
    let mut has_value = false;

    for row in &result_rows[frame_start..frame_end] {
        let value = eval_expr(arg_expr, row)?;
        if let Some(num) = extract_number(&value) {
            sum += num;
            has_value = true;
        }
    }

    // Return BigInt for integer sums, Double otherwise
    if has_value {
        // Check if sum is an integer
        if sum.fract() == 0.0 && sum.abs() <= i64::MAX as f64 {
            Ok(Literal::BigInt(sum as i64))
        } else {
            Ok(Literal::Double(sum))
        }
    } else {
        Ok(Literal::BigInt(0))
    }
}

/// Compute AVG over a window frame
pub(crate) fn compute_avg_over_frame(
    result_rows: &[Row],
    frame_start: usize,
    frame_end: usize,
    arg_expr: &raisin_sql::analyzer::TypedExpr,
) -> Result<Literal, ExecutionError> {
    let mut sum = 0.0;
    let mut count = 0;

    for row in &result_rows[frame_start..frame_end] {
        let value = eval_expr(arg_expr, row)?;
        if let Some(num) = extract_number(&value) {
            sum += num;
            count += 1;
        }
    }

    if count > 0 {
        Ok(Literal::Double(sum / count as f64))
    } else {
        Ok(Literal::Double(0.0))
    }
}

/// Compute MIN over a window frame
pub(crate) fn compute_min_over_frame(
    result_rows: &[Row],
    frame_start: usize,
    frame_end: usize,
    arg_expr: &raisin_sql::analyzer::TypedExpr,
) -> Result<Literal, ExecutionError> {
    let mut min_value: Option<Literal> = None;

    for row in &result_rows[frame_start..frame_end] {
        let value = eval_expr(arg_expr, row)?;

        if matches!(value, Literal::Null) {
            continue;
        }

        min_value = Some(match min_value {
            None => value,
            Some(current_min) => {
                if compare_literals(&value, &current_min) == Ordering::Less {
                    value
                } else {
                    current_min
                }
            }
        });
    }

    Ok(min_value.unwrap_or(Literal::Null))
}

/// Compute MAX over a window frame
pub(crate) fn compute_max_over_frame(
    result_rows: &[Row],
    frame_start: usize,
    frame_end: usize,
    arg_expr: &raisin_sql::analyzer::TypedExpr,
) -> Result<Literal, ExecutionError> {
    let mut max_value: Option<Literal> = None;

    for row in &result_rows[frame_start..frame_end] {
        let value = eval_expr(arg_expr, row)?;

        if matches!(value, Literal::Null) {
            continue;
        }

        max_value = Some(match max_value {
            None => value,
            Some(current_max) => {
                if compare_literals(&value, &current_max) == Ordering::Greater {
                    value
                } else {
                    current_max
                }
            }
        });
    }

    Ok(max_value.unwrap_or(Literal::Null))
}

/// Extract numeric value from a literal
pub(crate) fn extract_number(lit: &Literal) -> Option<f64> {
    match lit {
        Literal::Int(i) => Some(*i as f64),
        Literal::BigInt(i) => Some(*i as f64),
        Literal::Double(f) => Some(*f),
        _ => None,
    }
}
