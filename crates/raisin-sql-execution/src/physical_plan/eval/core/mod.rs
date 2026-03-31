//! Core expression evaluation logic
//!
//! This module contains the main `eval_expr` function that evaluates typed
//! expressions against a row of data at runtime.
//!
//! # Module Structure
//!
//! - `json_eval` - JSON operator evaluation (`->`, `->>`, `@>`, `?`, `#>`, etc.)

mod json_eval;

use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use raisin_error::Error;
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};

use super::binary_ops::{eval_binary_op, eval_unary_op};
use super::casting::cast_literal;
use super::functions::{eval_function, generate_function_column_name};
use super::helpers::{compare_literals, literals_equal};
use super::pattern::{sql_ilike_match, sql_like_match};

/// Evaluate a typed expression against a row
///
/// This function performs runtime evaluation of expressions, including:
/// - Column references
/// - Literals
/// - Binary operations (arithmetic, comparison, logical)
/// - Unary operations
/// - Function calls (DEPTH, PARENT, PATH_STARTS_WITH, etc.)
/// - JSON operations (->>, @>, <@)
///
/// # Errors
///
/// Returns an error if:
/// - Column not found in row
/// - Type mismatch in operations
/// - Invalid operation (e.g., division by zero)
/// - Function evaluation fails
pub fn eval_expr(expr: &TypedExpr, row: &Row) -> Result<Literal, Error> {
    match &expr.expr {
        Expr::Literal(lit) => Ok(lit.clone()),

        Expr::Column { table, column } => eval_column(table, column, row),

        Expr::BinaryOp { left, op, right } => eval_binary_op(left, op, right, row),

        Expr::UnaryOp { op, expr } => eval_unary_op(op, expr, row),

        Expr::Function {
            name,
            args,
            signature: _,
            filter,
        } => eval_function_expr(name, args, filter, row),

        Expr::IsNull { expr } => {
            let value = eval_expr(expr, row)?;
            Ok(Literal::Boolean(matches!(value, Literal::Null)))
        }

        Expr::IsNotNull { expr } => {
            let value = eval_expr(expr, row)?;
            Ok(Literal::Boolean(!matches!(value, Literal::Null)))
        }

        Expr::Between { expr, low, high } => {
            let value = eval_expr(expr, row)?;
            let low_val = eval_expr(low, row)?;
            let high_val = eval_expr(high, row)?;
            let ge_low = compare_literals(&value, &low_val, BinaryOperator::GtEq)?;
            let le_high = compare_literals(&value, &high_val, BinaryOperator::LtEq)?;
            Ok(Literal::Boolean(ge_low && le_high))
        }

        Expr::InList {
            expr,
            list,
            negated,
        } => eval_in_list(expr, list, *negated, row),

        Expr::InSubquery { .. } => Err(Error::Validation(
            "InSubquery expressions should be transformed to SemiJoin operators \
                 during logical plan building. If you see this error, the logical \
                 plan builder may not have processed the IN subquery correctly."
                .to_string(),
        )),

        Expr::Like {
            expr,
            pattern,
            negated,
        } => eval_like(expr, pattern, *negated, row),

        Expr::ILike {
            expr,
            pattern,
            negated,
        } => eval_ilike(expr, pattern, *negated, row),

        // JSON operators delegated to json_eval module
        Expr::JsonExtract { object, key } => json_eval::eval_json_extract(object, key, row),
        Expr::JsonExtractText { object, key } => {
            json_eval::eval_json_extract_text(object, key, row)
        }
        Expr::JsonContains { object, pattern } => {
            json_eval::eval_json_contains(object, pattern, row)
        }
        Expr::JsonKeyExists { object, key } => json_eval::eval_json_key_exists(object, key, row),
        Expr::JsonAnyKeyExists { object, keys } => {
            json_eval::eval_json_any_key_exists(object, keys, row)
        }
        Expr::JsonAllKeyExists { object, keys } => {
            json_eval::eval_json_all_key_exists(object, keys, row)
        }
        Expr::JsonExtractPath { object, path } => {
            json_eval::eval_json_extract_path(object, path, row)
        }
        Expr::JsonExtractPathText { object, path } => {
            json_eval::eval_json_extract_path_text(object, path, row)
        }
        Expr::JsonRemove { object, key } => json_eval::eval_json_remove(object, key, row),
        Expr::JsonRemoveAtPath { object, path } => {
            json_eval::eval_json_remove_at_path(object, path, row)
        }
        Expr::JsonPathMatch { object, path } => json_eval::eval_json_path_match(object, path, row),
        Expr::JsonPathExists { object, path } => {
            json_eval::eval_json_path_exists(object, path, row)
        }

        Expr::Cast { expr, target_type } => {
            let value = eval_expr(expr, row)?;
            cast_literal(value, target_type)
        }

        Expr::Case {
            conditions,
            else_expr,
        } => eval_case(conditions, else_expr.as_deref(), row),

        Expr::Window { function, .. } => eval_window(function, row),
    }
}

/// Evaluate a column reference against a row
fn eval_column(table: &str, column: &str, row: &Row) -> Result<Literal, Error> {
    // Strategy 1: Try qualified name (for pre-projection rows with known table)
    if !table.is_empty() {
        let qualified_name = format!("{}.{}", table, column);
        if let Some(value) = row.get(&qualified_name) {
            return from_property_value(value)
                .map_err(|e| Error::Validation(format!("Failed to convert column value: {}", e)));
        }
    }

    // Strategy 2: Try unqualified column name (for post-projection rows)
    if let Some(value) = row.get(column) {
        return from_property_value(value)
            .map_err(|e| Error::Validation(format!("Failed to convert column value: {}", e)));
    }

    // Strategy 3: Search for any column ending with ".{column}"
    if table.is_empty() {
        if let Some(value) = row.get_by_unqualified(column) {
            return from_property_value(value)
                .map_err(|e| Error::Validation(format!("Failed to convert column value: {}", e)));
        }
    }

    // Strategy 4: Final fallback - unqualified match regardless of table name
    if let Some(value) = row.get_by_unqualified(column) {
        return from_property_value(value)
            .map_err(|e| Error::Validation(format!("Failed to convert column value: {}", e)));
    }

    Ok(Literal::Null)
}

/// Evaluate a function expression, checking for pre-computed values first
fn eval_function_expr(
    name: &str,
    args: &[TypedExpr],
    filter: &Option<Box<TypedExpr>>,
    row: &Row,
) -> Result<Literal, Error> {
    let canonical_name = generate_function_column_name(name, args, filter);

    if let Some(value) = row.get(&canonical_name) {
        return from_property_value(value).map_err(|e| {
            Error::Validation(format!("Failed to convert pre-computed value: {}", e))
        });
    }

    eval_function(name, args, row)
}

/// Evaluate IN list expression
fn eval_in_list(
    expr: &TypedExpr,
    list: &[TypedExpr],
    negated: bool,
    row: &Row,
) -> Result<Literal, Error> {
    let value = eval_expr(expr, row)?;
    let mut found = false;

    for item in list {
        let item_val = eval_expr(item, row)?;
        if literals_equal(&value, &item_val)? {
            found = true;
            break;
        }
    }

    Ok(Literal::Boolean(if negated { !found } else { found }))
}

/// Evaluate LIKE expression
fn eval_like(
    expr: &TypedExpr,
    pattern: &TypedExpr,
    negated: bool,
    row: &Row,
) -> Result<Literal, Error> {
    let value = eval_expr(expr, row)?;
    let pattern_lit = eval_expr(pattern, row)?;

    match (&value, &pattern_lit) {
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        (Literal::Text(text), Literal::Text(pattern))
        | (Literal::Path(text), Literal::Text(pattern))
        | (Literal::Text(text), Literal::Path(pattern)) => {
            let matches = sql_like_match(text, pattern);
            Ok(Literal::Boolean(if negated { !matches } else { matches }))
        }
        _ => Err(Error::Validation(format!(
            "LIKE requires text arguments, got {:?} LIKE {:?}",
            value, pattern_lit
        ))),
    }
}

/// Evaluate ILIKE expression
fn eval_ilike(
    expr: &TypedExpr,
    pattern: &TypedExpr,
    negated: bool,
    row: &Row,
) -> Result<Literal, Error> {
    let value = eval_expr(expr, row)?;
    let pattern_lit = eval_expr(pattern, row)?;

    match (&value, &pattern_lit) {
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        (Literal::Text(text), Literal::Text(pattern))
        | (Literal::Path(text), Literal::Text(pattern))
        | (Literal::Text(text), Literal::Path(pattern)) => {
            let matches = sql_ilike_match(text, pattern);
            Ok(Literal::Boolean(if negated { !matches } else { matches }))
        }
        _ => Err(Error::Validation(format!(
            "ILIKE requires text arguments, got {:?} ILIKE {:?}",
            value, pattern_lit
        ))),
    }
}

/// Evaluate CASE expression
fn eval_case(
    conditions: &[(TypedExpr, TypedExpr)],
    else_expr: Option<&TypedExpr>,
    row: &Row,
) -> Result<Literal, Error> {
    for (condition, result) in conditions {
        let cond_value = eval_expr(condition, row)?;
        match cond_value {
            Literal::Boolean(true) => return eval_expr(result, row),
            Literal::Boolean(false) | Literal::Null => continue,
            _ => {
                return Err(Error::Validation(format!(
                    "CASE condition must evaluate to BOOLEAN, got {:?}",
                    cond_value
                )));
            }
        }
    }

    if let Some(else_result) = else_expr {
        eval_expr(else_result, row)
    } else {
        Ok(Literal::Null)
    }
}

/// Evaluate window function reference (pre-computed by Window operator)
fn eval_window(
    function: &raisin_sql::analyzer::WindowFunction,
    row: &Row,
) -> Result<Literal, Error> {
    let derived_name = match function {
        raisin_sql::analyzer::WindowFunction::RowNumber => "row_number",
        raisin_sql::analyzer::WindowFunction::Rank => "rank",
        raisin_sql::analyzer::WindowFunction::DenseRank => "dense_rank",
        raisin_sql::analyzer::WindowFunction::Count => "count",
        raisin_sql::analyzer::WindowFunction::Sum(_) => "sum",
        raisin_sql::analyzer::WindowFunction::Avg(_) => "avg",
        raisin_sql::analyzer::WindowFunction::Min(_) => "min",
        raisin_sql::analyzer::WindowFunction::Max(_) => "max",
    };

    if let Some(value) = row.get(derived_name) {
        return from_property_value(value).map_err(|e| {
            Error::Validation(format!("Failed to convert window function value: {}", e))
        });
    }

    Err(Error::Validation(format!(
        "Window function '{}' must be evaluated by Window operator before eval_expr. \
         Did you forget to add it to the Window operator?",
        derived_name
    )))
}
