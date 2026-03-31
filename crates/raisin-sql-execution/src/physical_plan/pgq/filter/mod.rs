//! WHERE Clause Evaluation
//!
//! Evaluates filter expressions against variable bindings.
//!
//! # Submodules
//!
//! - [`operators`] - Binary/unary operators, comparison, arithmetic
//! - [`property_access`] - Node and relationship property resolution
//! - [`functions`] - PGQ function evaluation (CARDINALITY, etc.)
//! - [`like_match`] - SQL LIKE pattern matching

mod functions;
mod like_match;
mod operators;
mod property_access;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use raisin_sql::ast::{BinaryOperator, Expr, Literal, UnaryOperator};
use raisin_storage::Storage;

use self::functions::evaluate_function;
use self::like_match::like_match;
use self::operators::{compare_values, evaluate_binary_op, evaluate_unary_op, values_equal};
use self::property_access::evaluate_property_access;
use super::context::PgqContext;
use super::types::{SqlValue, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Evaluate a WHERE expression against a binding
///
/// Returns true if the binding matches the filter.
pub async fn evaluate_where<S: Storage>(
    expr: &Expr,
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<bool> {
    let value = evaluate_expr(expr, binding, storage, context).await?;
    Ok(value.as_bool().unwrap_or(false))
}

/// Filter bindings by WHERE expression
///
/// Returns only bindings that match the filter.
pub async fn filter_bindings<S: Storage>(
    expr: &Expr,
    bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    let mut result = Vec::with_capacity(bindings.len());

    for mut binding in bindings {
        if evaluate_where(expr, &mut binding, storage, context).await? {
            result.push(binding);
        }
    }

    Ok(result)
}

/// Work item for iterative expression evaluation
enum EvalWork<'a> {
    /// Evaluate this expression and push result
    Eval(&'a Expr),
    /// Apply binary operator to top two values on stack
    ApplyBinaryOp(BinaryOperator),
    /// Apply unary operator to top value on stack
    ApplyUnaryOp(UnaryOperator),
    /// Apply IS NULL check
    ApplyIsNull { negated: bool },
    /// Apply LIKE check
    ApplyLike { pattern: String, negated: bool },
    /// Apply IN list check (list items are literals, evaluated inline)
    ApplyInList { list: Vec<&'a Expr>, negated: bool },
    /// Apply BETWEEN check (expects 3 values on stack: val, low, high)
    ApplyBetween { negated: bool },
    /// Apply JSON access operator (-> or ->>)
    ApplyJsonAccess { key: String, as_text: bool },
}

// NOTE: mod.rs slightly exceeds 300 lines (~335 lines) because the evaluate_expr
// function is a tightly coupled async iterative evaluator that dispatches to all
// submodules via a work/value stack. Splitting it further would fragment the core
// evaluation loop and hurt readability. This is intentional.

/// Evaluate an expression to a SqlValue (iterative to avoid async recursion)
pub async fn evaluate_expr<S: Storage>(
    expr: &Expr,
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let mut work_stack: Vec<EvalWork> = vec![EvalWork::Eval(expr)];
    let mut value_stack: Vec<SqlValue> = Vec::new();

    while let Some(work) = work_stack.pop() {
        match work {
            EvalWork::Eval(e) => match e {
                Expr::Literal(lit) => {
                    value_stack.push(evaluate_literal(lit));
                }

                Expr::PropertyAccess {
                    variable,
                    properties,
                    ..
                } => {
                    let val =
                        evaluate_property_access(variable, properties, binding, storage, context)
                            .await?;
                    value_stack.push(val);
                }

                Expr::BinaryOp {
                    left, op, right, ..
                } => {
                    work_stack.push(EvalWork::ApplyBinaryOp(*op));
                    work_stack.push(EvalWork::Eval(right));
                    work_stack.push(EvalWork::Eval(left));
                }

                Expr::UnaryOp {
                    op, expr: inner, ..
                } => {
                    work_stack.push(EvalWork::ApplyUnaryOp(*op));
                    work_stack.push(EvalWork::Eval(inner));
                }

                Expr::IsNull {
                    expr: inner,
                    negated,
                    ..
                } => {
                    work_stack.push(EvalWork::ApplyIsNull { negated: *negated });
                    work_stack.push(EvalWork::Eval(inner));
                }

                Expr::InList {
                    expr: inner,
                    list,
                    negated,
                    ..
                } => {
                    let list_refs: Vec<&Expr> = list.iter().collect();
                    work_stack.push(EvalWork::ApplyInList {
                        list: list_refs,
                        negated: *negated,
                    });
                    work_stack.push(EvalWork::Eval(inner));
                }

                Expr::Like {
                    expr: inner,
                    pattern,
                    negated,
                    ..
                } => {
                    work_stack.push(EvalWork::ApplyLike {
                        pattern: pattern.clone(),
                        negated: *negated,
                    });
                    work_stack.push(EvalWork::Eval(inner));
                }

                Expr::Between {
                    expr: inner,
                    low,
                    high,
                    negated,
                    ..
                } => {
                    work_stack.push(EvalWork::ApplyBetween { negated: *negated });
                    work_stack.push(EvalWork::Eval(high));
                    work_stack.push(EvalWork::Eval(low));
                    work_stack.push(EvalWork::Eval(inner));
                }

                Expr::Nested(inner) => {
                    work_stack.push(EvalWork::Eval(inner));
                }

                Expr::JsonAccess {
                    expr, key, as_text, ..
                } => {
                    work_stack.push(EvalWork::ApplyJsonAccess {
                        key: key.clone(),
                        as_text: *as_text,
                    });
                    work_stack.push(EvalWork::Eval(expr));
                }

                Expr::JsonPathAccess { variable, path, .. } => {
                    let val =
                        evaluate_property_access(variable, path, binding, storage, context).await?;
                    value_stack.push(val);
                }

                Expr::FunctionCall { name, args, .. } => {
                    let val = evaluate_function(name, args, binding)?;
                    value_stack.push(val);
                }

                _ => {
                    return Err(ExecutionError::Validation(format!(
                        "Unsupported expression in WHERE: {:?}",
                        e
                    )));
                }
            },

            EvalWork::ApplyBinaryOp(op) => {
                let right = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in binary op".into())
                })?;
                let left = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in binary op".into())
                })?;
                let result = evaluate_binary_op(op, left, right)?;
                value_stack.push(result);
            }

            EvalWork::ApplyUnaryOp(op) => {
                let val = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in unary op".into())
                })?;
                let result = evaluate_unary_op(op, val)?;
                value_stack.push(result);
            }

            EvalWork::ApplyIsNull { negated } => {
                let val = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in IS NULL".into())
                })?;
                let is_null = val.is_null();
                value_stack.push(SqlValue::Boolean(if negated { !is_null } else { is_null }));
            }

            EvalWork::ApplyLike { pattern, negated } => {
                let val = value_stack
                    .pop()
                    .ok_or_else(|| ExecutionError::Validation("Stack underflow in LIKE".into()))?;
                let matches = if let SqlValue::String(s) = val {
                    like_match(&s, &pattern)
                } else {
                    false
                };
                value_stack.push(SqlValue::Boolean(if negated { !matches } else { matches }));
            }

            EvalWork::ApplyInList { list, negated } => {
                let val = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in IN list".into())
                })?;

                let mut found = false;
                for item in list {
                    let item_val = match item {
                        Expr::Literal(lit) => evaluate_literal(lit),
                        Expr::PropertyAccess {
                            variable,
                            properties,
                            ..
                        } => {
                            evaluate_property_access(
                                variable, properties, binding, storage, context,
                            )
                            .await?
                        }
                        _ => {
                            return Err(ExecutionError::Validation(
                                "Complex expressions in IN list not supported".into(),
                            ));
                        }
                    };
                    if values_equal(&val, &item_val) {
                        found = true;
                        break;
                    }
                }
                value_stack.push(SqlValue::Boolean(if negated { !found } else { found }));
            }

            EvalWork::ApplyBetween { negated } => {
                let high_val = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in BETWEEN (high)".into())
                })?;
                let low_val = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in BETWEEN (low)".into())
                })?;
                let val = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in BETWEEN (val)".into())
                })?;

                let in_range = compare_values(&val, &low_val)
                    .map(|o| o != std::cmp::Ordering::Less)
                    .unwrap_or(false)
                    && compare_values(&val, &high_val)
                        .map(|o| o != std::cmp::Ordering::Greater)
                        .unwrap_or(false);

                value_stack.push(SqlValue::Boolean(if negated {
                    !in_range
                } else {
                    in_range
                }));
            }

            EvalWork::ApplyJsonAccess { key, as_text } => {
                let base = value_stack.pop().ok_or_else(|| {
                    ExecutionError::Validation("Stack underflow in JSON access".into())
                })?;

                let result = match &base {
                    SqlValue::Json(json) => match json.get(&key) {
                        Some(v) if as_text => match v {
                            serde_json::Value::String(s) => SqlValue::String(s.clone()),
                            serde_json::Value::Null => SqlValue::Null,
                            other => SqlValue::String(other.to_string()),
                        },
                        Some(v) => SqlValue::Json(v.clone()),
                        None => SqlValue::Null,
                    },
                    _ => SqlValue::Null,
                };
                value_stack.push(result);
            }
        }
    }

    value_stack.pop().ok_or_else(|| {
        ExecutionError::Validation("Expression evaluation produced no result".into())
    })
}

/// Evaluate a literal to SqlValue
fn evaluate_literal(lit: &Literal) -> SqlValue {
    match lit {
        Literal::String(s) => SqlValue::String(s.clone()),
        Literal::Integer(i) => SqlValue::Integer(*i),
        Literal::Float(f) => SqlValue::Float(*f),
        Literal::Boolean(b) => SqlValue::Boolean(*b),
        Literal::Null => SqlValue::Null,
    }
}
