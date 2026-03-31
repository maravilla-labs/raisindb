//! Async expression evaluator for RELATES expressions

use super::context::EvalContext;
use super::resolver::RelationResolver;
use crate::ast::{BinOp, Expr, UnOp};
use crate::error::EvalError;
use crate::value::Value;

/// Check if an expression requires async evaluation (contains RELATES)
pub fn requires_async(expr: &Expr) -> bool {
    match expr {
        Expr::Relates { .. } => true,
        Expr::BinaryOp { left, right, .. } => requires_async(left) || requires_async(right),
        Expr::UnaryOp { expr, .. } => requires_async(expr),
        Expr::PropertyAccess { object, .. } => requires_async(object),
        Expr::IndexAccess { object, index } => requires_async(object) || requires_async(index),
        Expr::MethodCall { object, args, .. } => {
            requires_async(object) || args.iter().any(requires_async)
        }
        Expr::Grouped(inner) => requires_async(inner),
        Expr::Literal(_) | Expr::Variable(_) => false,
    }
}

/// Evaluate an expression asynchronously with a relation resolver
pub async fn evaluate_async(
    expr: &Expr,
    ctx: &EvalContext,
    resolver: &dyn RelationResolver,
) -> Result<Value, EvalError> {
    match expr {
        // Handle RELATES expression asynchronously
        Expr::Relates {
            source,
            target,
            relation_types,
            min_depth,
            max_depth,
            direction,
        } => {
            // Evaluate source and target to get their IDs
            let source_val = Box::pin(evaluate_async(source, ctx, resolver)).await?;
            let target_val = Box::pin(evaluate_async(target, ctx, resolver)).await?;

            let source_id = source_val.as_str().ok_or_else(|| {
                EvalError::type_error("RELATES", "string", source_val.type_name())
            })?;

            let target_id = target_val.as_str().ok_or_else(|| {
                EvalError::type_error("RELATES", "string", target_val.type_name())
            })?;

            // Call the resolver to check if path exists
            let has_path = resolver
                .has_path(
                    source_id,
                    target_id,
                    relation_types,
                    *min_depth,
                    *max_depth,
                    *direction,
                )
                .await?;

            Ok(Value::Boolean(has_path))
        }

        // For binary operations, short-circuit evaluation for AND/OR
        Expr::BinaryOp { left, op, right } => match op {
            BinOp::And => {
                let left_val = Box::pin(evaluate_async(left, ctx, resolver)).await?;
                if !left_val.is_truthy() {
                    return Ok(Value::Boolean(false));
                }
                let right_val = Box::pin(evaluate_async(right, ctx, resolver)).await?;
                Ok(Value::Boolean(right_val.is_truthy()))
            }
            BinOp::Or => {
                let left_val = Box::pin(evaluate_async(left, ctx, resolver)).await?;
                if left_val.is_truthy() {
                    return Ok(Value::Boolean(true));
                }
                let right_val = Box::pin(evaluate_async(right, ctx, resolver)).await?;
                Ok(Value::Boolean(right_val.is_truthy()))
            }
            _ => {
                // For other binary ops, evaluate both sides
                let left_val = Box::pin(evaluate_async(left, ctx, resolver)).await?;
                let right_val = Box::pin(evaluate_async(right, ctx, resolver)).await?;

                // Delegate to sync evaluator's binary op logic for non-logical ops
                super::evaluator::eval_binary_values(&left_val, *op, &right_val)
            }
        },

        // For unary operations
        Expr::UnaryOp { op, expr: inner } => {
            let value = Box::pin(evaluate_async(inner, ctx, resolver)).await?;
            match op {
                UnOp::Not => Ok(Value::Boolean(!value.is_truthy())),
                UnOp::Neg => match value {
                    Value::Integer(i) => Ok(Value::Integer(-i)),
                    Value::Float(f) => Ok(Value::Float(-f)),
                    other => Err(EvalError::type_error(
                        "negation",
                        "number",
                        other.type_name(),
                    )),
                },
            }
        }

        // For property and index access
        Expr::PropertyAccess { object, property } => {
            let obj_value = Box::pin(evaluate_async(object, ctx, resolver)).await?;
            match &obj_value {
                Value::Object(map) => Ok(map.get(property).cloned().unwrap_or(Value::Null)),
                Value::Null => Ok(Value::Null),
                _ => Ok(Value::Null),
            }
        }

        Expr::IndexAccess { object, index } => {
            let obj_value = Box::pin(evaluate_async(object, ctx, resolver)).await?;
            let index_value = Box::pin(evaluate_async(index, ctx, resolver)).await?;

            match &obj_value {
                Value::Array(arr) => {
                    let idx = index_value.as_integer().ok_or_else(|| {
                        EvalError::InvalidIndexType(index_value.type_name().to_string())
                    })?;

                    if idx < 0 {
                        return Err(EvalError::index_out_of_bounds(idx, arr.len()));
                    }

                    arr.get(idx as usize)
                        .cloned()
                        .ok_or_else(|| EvalError::index_out_of_bounds(idx, arr.len()))
                }
                Value::Object(map) => {
                    let key = index_value.as_str().ok_or_else(|| {
                        EvalError::InvalidIndexType(index_value.type_name().to_string())
                    })?;

                    map.get(key)
                        .cloned()
                        .ok_or_else(|| EvalError::property_not_found(key, "object"))
                }
                Value::Null => Ok(Value::Null),
                other => Err(EvalError::type_error(
                    "index access",
                    "array or object",
                    other.type_name(),
                )),
            }
        }

        // For method calls - eval synchronously since methods don't need async
        Expr::MethodCall {
            object,
            method: _method,
            args,
        } => {
            // For method calls, use the sync evaluator since methods themselves are sync
            // We only need async for traversing to get to the method call
            let obj_val = Box::pin(evaluate_async(object, ctx, resolver)).await?;
            if obj_val == Value::Null {
                return Ok(Value::Null);
            }

            // Build a temporary method call expression with evaluated object
            // This is a bit hacky but avoids duplicating all the method logic
            // In practice, args should be simple enough that sync eval works
            let mut _arg_values = Vec::new();
            for arg in args {
                _arg_values.push(Box::pin(evaluate_async(arg, ctx, resolver)).await?);
            }

            // We need to call the method - but we can't easily reuse eval_method
            // So let's just delegate to the sync evaluator for the final method call
            // This works because by this point we've resolved any async parts

            // Actually, we need to rebuild an expression or duplicate the method logic
            // For now, let's use evaluate on a literal and handle methods separately
            // This is a limitation - method calls in async contexts need more work
            Err(EvalError::type_error(
                "method call in async context",
                "simple property access",
                "method call - not yet fully supported in async evaluation",
            ))
        }

        Expr::Grouped(inner) => Box::pin(evaluate_async(inner, ctx, resolver)).await,

        // Delegate simple expressions to the synchronous evaluator
        Expr::Literal(_) | Expr::Variable(_) => super::evaluator::evaluate(expr, ctx),
    }
}
