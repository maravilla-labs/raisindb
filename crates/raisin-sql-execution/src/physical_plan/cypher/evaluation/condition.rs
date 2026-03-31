//! WHERE clause evaluation for Cypher
//!
//! Provides functions to evaluate boolean conditions and filter bindings.

use raisin_cypher_parser::{BinOp, Expr};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use super::expr::evaluate_expr_async_impl;
use super::functions::FunctionContext;
use crate::physical_plan::cypher::types::VariableBinding;
use crate::physical_plan::executor::ExecutionError;

/// Execute WHERE clause (filter bindings)
///
/// Evaluates a boolean condition for each binding and returns only bindings
/// where the condition evaluates to true.
///
/// # Arguments
///
/// * `condition` - Boolean expression to evaluate
/// * `bindings` - Input bindings to filter
/// * `context` - Function evaluation context
///
/// # Returns
///
/// Result containing filtered bindings or an ExecutionError
///
/// # Example
///
/// ```ignore
/// // WHERE n.age > 18
/// let filtered = execute_where(&condition, bindings, &context).await?;
/// ```
pub async fn execute_where<S: Storage>(
    condition: &Expr,
    bindings: Vec<VariableBinding>,
    context: &FunctionContext<'_, S>,
) -> Result<Vec<VariableBinding>, ExecutionError> {
    let mut result = Vec::new();

    for binding in bindings {
        if evaluate_condition(condition, &binding, context).await? {
            result.push(binding);
        }
    }

    Ok(result)
}

/// Evaluate a boolean condition
///
/// Recursively evaluates boolean expressions including:
/// - Logical operators (AND, OR)
/// - Comparison operators (=, !=, <, >, <=, >=)
///
/// # Arguments
///
/// * `expr` - Expression to evaluate (must be boolean)
/// * `binding` - Current variable binding
/// * `context` - Function evaluation context
///
/// # Returns
///
/// Result containing true/false or an ExecutionError
pub async fn evaluate_condition<'a, S: Storage + 'a>(
    expr: &'a Expr,
    binding: &'a VariableBinding,
    context: &'a FunctionContext<'a, S>,
) -> Result<bool, ExecutionError> {
    enum LogicalFrame<'a> {
        AwaitRight { op: BinOp, right: &'a Expr },
        Combine { op: BinOp, left_result: bool },
    }

    let mut stack: Vec<LogicalFrame<'a>> = Vec::new();
    let mut current_expr: Option<&'a Expr> = Some(expr);
    let mut pending_result: Option<bool> = None;

    loop {
        if let Some(expr_to_eval) = current_expr.take() {
            match expr_to_eval {
                Expr::BinaryOp { left, op, right } => match op {
                    BinOp::And | BinOp::Or => {
                        stack.push(LogicalFrame::AwaitRight { op: *op, right });
                        current_expr = Some(left);
                    }
                    _ => {
                        pending_result =
                            Some(evaluate_binary_op(*op, left, right, binding, context).await?);
                    }
                },
                Expr::Literal(raisin_cypher_parser::Literal::Boolean(b)) => {
                    pending_result = Some(*b);
                }
                _ => {
                    return Err(ExecutionError::Validation(
                        "Invalid WHERE condition - expected boolean expression".to_string(),
                    ));
                }
            }
        } else if let Some(value) = pending_result.take() {
            if let Some(frame) = stack.pop() {
                match frame {
                    LogicalFrame::AwaitRight { op, right } => match op {
                        BinOp::And => {
                            if !value {
                                pending_result = Some(false);
                            } else {
                                stack.push(LogicalFrame::Combine {
                                    op,
                                    left_result: value,
                                });
                                current_expr = Some(right);
                            }
                        }
                        BinOp::Or => {
                            if value {
                                pending_result = Some(true);
                            } else {
                                stack.push(LogicalFrame::Combine {
                                    op,
                                    left_result: value,
                                });
                                current_expr = Some(right);
                            }
                        }
                        _ => unreachable!(),
                    },
                    LogicalFrame::Combine { op, left_result } => {
                        let combined = match op {
                            BinOp::And => left_result && value,
                            BinOp::Or => left_result || value,
                            _ => unreachable!(),
                        };
                        pending_result = Some(combined);
                    }
                }
            } else {
                return Ok(value);
            }
        } else {
            break;
        }
    }

    Err(ExecutionError::Validation(
        "Invalid WHERE condition - expected boolean expression".to_string(),
    ))
}

async fn evaluate_binary_op<'a, S: Storage + 'a>(
    op: BinOp,
    left: &'a Expr,
    right: &'a Expr,
    binding: &'a VariableBinding,
    context: &'a FunctionContext<'a, S>,
) -> Result<bool, ExecutionError> {
    match op {
        // Equality: evaluate both sides and compare
        BinOp::Eq => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;
            let matches = left_val == right_val;
            tracing::debug!(
                "WHERE Eq comparison: {:?} == {:?} => {}",
                left_val,
                right_val,
                matches
            );
            Ok(matches)
        }

        // Inequality: evaluate both sides and compare
        BinOp::Neq => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;
            Ok(left_val != right_val)
        }

        // Less than: requires numeric operands (handles Integer and Float)
        BinOp::Lt => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;

            match (left_val, right_val) {
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l < r),
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok((l as f64) < r),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l < (r as f64)),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok(l < r),
                _ => Err(ExecutionError::Validation(
                    "Less than comparison requires numeric operands".to_string(),
                )),
            }
        }

        // Less than or equal: requires numeric operands (handles Integer and Float)
        BinOp::Lte => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;

            match (left_val, right_val) {
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l <= r),
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok((l as f64) <= r),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l <= (r as f64)),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok(l <= r),
                _ => Err(ExecutionError::Validation(
                    "Less than or equal comparison requires numeric operands".to_string(),
                )),
            }
        }

        // Greater than: requires numeric operands (handles Integer and Float)
        BinOp::Gt => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;

            match (left_val, right_val) {
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l > r),
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok((l as f64) > r),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l > (r as f64)),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok(l > r),
                _ => Err(ExecutionError::Validation(
                    "Greater than comparison requires numeric operands".to_string(),
                )),
            }
        }

        // Greater than or equal: requires numeric operands (handles Integer and Float)
        BinOp::Gte => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;

            match (left_val, right_val) {
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l >= r),
                (
                    raisin_models::nodes::properties::PropertyValue::Integer(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok((l as f64) >= r),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Integer(r),
                ) => Ok(l >= (r as f64)),
                (
                    raisin_models::nodes::properties::PropertyValue::Float(l),
                    raisin_models::nodes::properties::PropertyValue::Float(r),
                ) => Ok(l >= r),
                _ => Err(ExecutionError::Validation(
                    "Greater than or equal comparison requires numeric operands".to_string(),
                )),
            }
        }

        // String operations: STARTS WITH / ENDS WITH / CONTAINS
        BinOp::StartsWith | BinOp::EndsWith | BinOp::Contains => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;

            match (&left_val, &right_val) {
                (PropertyValue::String(text), PropertyValue::String(pattern)) => {
                    let matches = match op {
                        BinOp::StartsWith => text.starts_with(pattern),
                        BinOp::EndsWith => text.ends_with(pattern),
                        BinOp::Contains => text.contains(pattern),
                        _ => unreachable!(),
                    };
                    Ok(matches)
                }
                _ => Err(ExecutionError::Validation(format!(
                    "{} operator requires string operands",
                    op
                ))),
            }
        }

        // Collection IN operator: value IN [list]
        BinOp::In => {
            let left_val = evaluate_expr_async_impl(left, binding, context).await?;
            let right_val = evaluate_expr_async_impl(right, binding, context).await?;

            match right_val {
                PropertyValue::Array(list) => {
                    // Check if left value is in the list
                    Ok(list.contains(&left_val))
                }
                _ => Err(ExecutionError::Validation(
                    "IN operator requires a list on the right side".to_string(),
                )),
            }
        }

        _ => Err(ExecutionError::Validation(format!(
            "Unsupported operator in WHERE clause: {:?}",
            op
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_condition_evaluation() {
        // Tests would require setting up context and binding
        // This is a placeholder for actual unit tests
    }
}
