//! PGQ Aggregate Functions
//!
//! Implements aggregate functions for GRAPH_TABLE COLUMNS clause:
//! - COUNT(*) - Count matching rows
//! - COLLECT() - Gather values into an array

use std::sync::Arc;

use raisin_sql::ast::{ColumnExpr, Expr};
use raisin_storage::Storage;

use super::context::PgqContext;
use super::filter::evaluate_expr;
use super::types::{SqlValue, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Check if an expression is an aggregate function
pub fn is_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, .. } => {
            let name_lower = name.to_lowercase();
            matches!(
                name_lower.as_str(),
                "count" | "sum" | "avg" | "min" | "max" | "collect" | "array_agg"
            )
        }
        _ => false,
    }
}

/// Check if any column expression contains an aggregate
pub fn has_aggregates(columns: &[ColumnExpr]) -> bool {
    columns.iter().any(|c| contains_aggregate(&c.expr))
}

/// Check if an expression tree contains an aggregate
fn contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, args, .. } => {
            let name_lower = name.to_lowercase();
            if matches!(
                name_lower.as_str(),
                "count" | "sum" | "avg" | "min" | "max" | "collect" | "array_agg"
            ) {
                return true;
            }
            args.iter().any(contains_aggregate)
        }
        Expr::BinaryOp { left, right, .. } => contains_aggregate(left) || contains_aggregate(right),
        Expr::UnaryOp { expr, .. } => contains_aggregate(expr),
        Expr::Nested(inner) => contains_aggregate(inner),
        _ => false,
    }
}

/// Evaluate COUNT(*) or COUNT(expr)
pub async fn evaluate_count<S: Storage>(
    args: &[Expr],
    distinct: bool,
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    if args.is_empty() {
        // COUNT(*) - count all rows
        return Ok(SqlValue::Integer(bindings.len() as i64));
    }

    // COUNT(expr) - count non-null values
    let mut count = 0i64;
    let mut seen_values: Vec<SqlValue> = Vec::new();

    for binding in bindings.iter_mut() {
        let val = evaluate_expr(&args[0], binding, storage, context).await?;
        if !val.is_null() {
            if distinct {
                // For DISTINCT, check if we've seen this value
                if !seen_values.iter().any(|v| values_equal(v, &val)) {
                    seen_values.push(val);
                    count += 1;
                }
            } else {
                count += 1;
            }
        }
    }

    Ok(SqlValue::Integer(count))
}

/// Evaluate COLLECT(expr) - gather values into an array
pub async fn evaluate_collect<S: Storage>(
    args: &[Expr],
    distinct: bool,
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    if args.is_empty() {
        return Err(ExecutionError::Validation(
            "COLLECT requires an argument".into(),
        ));
    }

    let mut values: Vec<SqlValue> = Vec::with_capacity(bindings.len());
    let mut seen: Vec<SqlValue> = Vec::new();

    for binding in bindings.iter_mut() {
        let val = evaluate_expr(&args[0], binding, storage, context).await?;
        if !val.is_null() {
            if distinct {
                if !seen.iter().any(|v| values_equal(v, &val)) {
                    seen.push(val.clone());
                    values.push(val);
                }
            } else {
                values.push(val);
            }
        }
    }

    Ok(SqlValue::Array(values))
}

/// Evaluate SUM(expr)
pub async fn evaluate_sum<S: Storage>(
    args: &[Expr],
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    if args.is_empty() {
        return Err(ExecutionError::Validation(
            "SUM requires an argument".into(),
        ));
    }

    let mut sum_int: Option<i64> = None;
    let mut sum_float: Option<f64> = None;

    for binding in bindings.iter_mut() {
        let val = evaluate_expr(&args[0], binding, storage, context).await?;
        match val {
            SqlValue::Integer(i) => {
                sum_int = Some(sum_int.unwrap_or(0) + i);
            }
            SqlValue::Float(f) => {
                sum_float = Some(sum_float.unwrap_or(0.0) + f);
            }
            SqlValue::Null => {}
            _ => {
                return Err(ExecutionError::Validation(
                    "SUM requires numeric values".into(),
                ))
            }
        }
    }

    // If we have floats, convert everything to float
    if let Some(f) = sum_float {
        let total = f + sum_int.map(|i| i as f64).unwrap_or(0.0);
        Ok(SqlValue::Float(total))
    } else if let Some(i) = sum_int {
        Ok(SqlValue::Integer(i))
    } else {
        Ok(SqlValue::Null)
    }
}

/// Evaluate AVG(expr)
pub async fn evaluate_avg<S: Storage>(
    args: &[Expr],
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    if args.is_empty() {
        return Err(ExecutionError::Validation(
            "AVG requires an argument".into(),
        ));
    }

    let mut sum: f64 = 0.0;
    let mut count: i64 = 0;

    for binding in bindings.iter_mut() {
        let val = evaluate_expr(&args[0], binding, storage, context).await?;
        match val {
            SqlValue::Integer(i) => {
                sum += i as f64;
                count += 1;
            }
            SqlValue::Float(f) => {
                sum += f;
                count += 1;
            }
            SqlValue::Null => {}
            _ => {
                return Err(ExecutionError::Validation(
                    "AVG requires numeric values".into(),
                ))
            }
        }
    }

    if count == 0 {
        Ok(SqlValue::Null)
    } else {
        Ok(SqlValue::Float(sum / count as f64))
    }
}

/// Evaluate MIN(expr)
pub async fn evaluate_min<S: Storage>(
    args: &[Expr],
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    if args.is_empty() {
        return Err(ExecutionError::Validation(
            "MIN requires an argument".into(),
        ));
    }

    let mut min: Option<SqlValue> = None;

    for binding in bindings.iter_mut() {
        let val = evaluate_expr(&args[0], binding, storage, context).await?;
        if val.is_null() {
            continue;
        }

        min = Some(match min {
            None => val,
            Some(current) => {
                if compare_values(&val, &current) == Some(std::cmp::Ordering::Less) {
                    val
                } else {
                    current
                }
            }
        });
    }

    Ok(min.unwrap_or(SqlValue::Null))
}

/// Evaluate MAX(expr)
pub async fn evaluate_max<S: Storage>(
    args: &[Expr],
    bindings: &mut [VariableBinding],
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    if args.is_empty() {
        return Err(ExecutionError::Validation(
            "MAX requires an argument".into(),
        ));
    }

    let mut max: Option<SqlValue> = None;

    for binding in bindings.iter_mut() {
        let val = evaluate_expr(&args[0], binding, storage, context).await?;
        if val.is_null() {
            continue;
        }

        max = Some(match max {
            None => val,
            Some(current) => {
                if compare_values(&val, &current) == Some(std::cmp::Ordering::Greater) {
                    val
                } else {
                    current
                }
            }
        });
    }

    Ok(max.unwrap_or(SqlValue::Null))
}

/// Compare two SqlValues for ordering
fn compare_values(left: &SqlValue, right: &SqlValue) -> Option<std::cmp::Ordering> {
    match (left, right) {
        (SqlValue::Integer(a), SqlValue::Integer(b)) => Some(a.cmp(b)),
        (SqlValue::Float(a), SqlValue::Float(b)) => a.partial_cmp(b),
        (SqlValue::Integer(a), SqlValue::Float(b)) => (*a as f64).partial_cmp(b),
        (SqlValue::Float(a), SqlValue::Integer(b)) => a.partial_cmp(&(*b as f64)),
        (SqlValue::String(a), SqlValue::String(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// Check value equality
fn values_equal(left: &SqlValue, right: &SqlValue) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::ast::SourceSpan;

    fn make_count_star() -> Expr {
        Expr::FunctionCall {
            name: "COUNT".into(),
            args: vec![Expr::Wildcard {
                qualifier: None,
                span: SourceSpan::empty(),
            }],
            distinct: false,
            span: SourceSpan::empty(),
        }
    }

    fn make_collect(var: &str, prop: &str) -> Expr {
        Expr::FunctionCall {
            name: "COLLECT".into(),
            args: vec![Expr::PropertyAccess {
                variable: var.into(),
                properties: vec![prop.into()],
                span: SourceSpan::empty(),
            }],
            distinct: false,
            span: SourceSpan::empty(),
        }
    }

    #[test]
    fn test_is_aggregate() {
        assert!(is_aggregate(&make_count_star()));
        assert!(is_aggregate(&make_collect("n", "name")));
        assert!(!is_aggregate(&Expr::Literal(
            raisin_sql::ast::Literal::Integer(42)
        )));
    }

    #[test]
    fn test_has_aggregates() {
        let columns = vec![ColumnExpr {
            expr: make_count_star(),
            alias: Some("cnt".into()),
            span: SourceSpan::empty(),
        }];
        assert!(has_aggregates(&columns));

        let columns = vec![ColumnExpr {
            expr: Expr::PropertyAccess {
                variable: "n".into(),
                properties: vec!["name".into()],
                span: SourceSpan::empty(),
            },
            alias: None,
            span: SourceSpan::empty(),
        }];
        assert!(!has_aggregates(&columns));
    }
}
