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
use super::helpers::{comparison_op, logical_and, logical_or};
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
            // Three-valued: a NULL bound or operand makes the half unknown.
            let ge_low = comparison_op(&value, BinaryOperator::GtEq, &low_val)?;
            let le_high = comparison_op(&value, BinaryOperator::LtEq, &high_val)?;
            logical_and(&ge_low, &le_high)
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

        Expr::Exists { .. } | Expr::ScalarSubquery { .. } | Expr::QuantifiedSubquery { .. } => {
            Err(Error::Validation(
                "subquery reached the executor unbound: the engine evaluates EXISTS / scalar / \
                 ANY-ALL subqueries before planning (engine::subquery_bind); this statement \
                 path skipped that step"
                    .to_string(),
            ))
        }

        Expr::Quantified {
            left,
            op,
            right,
            all,
        } => super::regex_ops::eval_quantified(left, *op, right, *all, row),

        Expr::Regex {
            expr,
            pattern,
            case_insensitive,
            negated,
            similar_to,
        } => super::regex_ops::eval_regex(
            expr,
            pattern,
            *case_insensitive,
            *negated,
            *similar_to,
            row,
        ),

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
    match column_value(table, column, row) {
        Some(value) => from_property_value(value)
            .map_err(|e| Error::Validation(format!("Failed to convert column value: {}", e))),
        None => Ok(Literal::Null),
    }
}

/// The stored value a column reference names, without converting it.
///
/// THE lookup `eval_column` does, exposed so an operator that needs one part of
/// a column — `properties ->> 'title'` — can take that part instead of
/// converting the whole column first.
pub(crate) fn column_value<'a>(
    table: &str,
    column: &str,
    row: &'a Row,
) -> Option<&'a raisin_models::nodes::properties::PropertyValue> {
    column_index(table, column, row).map(|i| &row.columns[i])
}

/// The position in `row` of the column a reference names — THE lookup behind
/// [`column_value`], without allocating the qualified name.
pub(crate) fn column_index(table: &str, column: &str, row: &Row) -> Option<usize> {
    // Strategy 1: Try qualified name (for pre-projection rows with known table)
    if !table.is_empty() {
        let mut buf = [0u8; 256];
        let len = table.len() + 1 + column.len();
        let found = if len <= buf.len() {
            buf[..table.len()].copy_from_slice(table.as_bytes());
            buf[table.len()] = b'.';
            buf[table.len() + 1..len].copy_from_slice(column.as_bytes());
            // Both halves are UTF-8, so their concatenation is.
            std::str::from_utf8(&buf[..len])
                .ok()
                .and_then(|qualified| row.columns.get_index_of(qualified))
        } else {
            row.columns
                .get_index_of(format!("{table}.{column}").as_str())
        };
        if found.is_some() {
            return found;
        }
    }

    // Strategy 2: unqualified name (post-projection rows); strategies 3 and
    // 4 (a column ending with ".{column}", whatever the table name) are the
    // same lookup.
    row.index_of_unqualified(column)
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
    // Three-valued (`x IN (a, b)` is `x = a OR x = b`): a match is TRUE;
    // no match is FALSE unless some comparison was unknown (a NULL operand or
    // a NULL item), which makes it NULL — so `NULL NOT IN (…)` and
    // `x NOT IN (…, NULL)` never come out TRUE.
    let value = eval_expr(expr, row)?;
    let mut result = Literal::Boolean(false);
    for item in list {
        let item_val = eval_expr(item, row)?;
        result = logical_or(
            &result,
            &comparison_op(&value, BinaryOperator::Eq, &item_val)?,
        )?;
        if result == Literal::Boolean(true) {
            break;
        }
    }
    match result {
        Literal::Boolean(found) => Ok(Literal::Boolean(found != negated)),
        other => Ok(other),
    }
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
