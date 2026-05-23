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

        // Method calls: the method itself is synchronous, but its receiver or
        // arguments may contain RELATES (e.g. `… || auth.groups.contains("x")`
        // routed async because a sibling clause has RELATES). Async-resolve the
        // receiver + args, then dispatch via the shared sync method dispatch.
        // (BUG-1: this arm used to return an error, which fails closed to Deny
        // for any policy mixing RELATES with a method call.)
        Expr::MethodCall {
            object,
            method,
            args,
        } => {
            let obj_val = Box::pin(evaluate_async(object, ctx, resolver)).await?;
            // Null-safe: don't evaluate args on a null receiver.
            if obj_val == Value::Null {
                return Ok(Value::Null);
            }

            let mut arg_vals = Vec::with_capacity(args.len());
            for arg in args {
                arg_vals.push(Box::pin(evaluate_async(arg, ctx, resolver)).await?);
            }

            super::evaluator::eval_method_on_values(&obj_val, method, &arg_vals)
        }

        Expr::Grouped(inner) => Box::pin(evaluate_async(inner, ctx, resolver)).await,

        // Delegate simple expressions to the synchronous evaluator
        Expr::Literal(_) | Expr::Variable(_) => super::evaluator::evaluate(expr, ctx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::resolver::NoOpResolver;
    use crate::parser::parse;

    fn ctx(v: serde_json::Value) -> EvalContext {
        EvalContext::from_json(v).expect("valid context json")
    }

    /// A resolver whose `has_path` always returns a fixed answer — lets a test
    /// drive the RELATES side without a real graph.
    struct FixedResolver(bool);

    #[async_trait::async_trait]
    impl RelationResolver for FixedResolver {
        async fn has_path(
            &self,
            _source_id: &str,
            _target_id: &str,
            _relation_types: &[String],
            _min_depth: u32,
            _max_depth: u32,
            _direction: crate::ast::RelDirection,
        ) -> Result<bool, EvalError> {
            Ok(self.0)
        }
    }

    async fn eval(expr: &str, v: serde_json::Value, resolver: &dyn RelationResolver) -> Value {
        let parsed = parse(expr).expect("parse");
        evaluate_async(&parsed, &ctx(v), resolver)
            .await
            .expect("evaluate_async should not error")
    }

    #[tokio::test]
    async fn bug1_relates_or_method_no_longer_errors() {
        // The exact BUG-1 shape: RELATES `||` a method call. The whole tree
        // routes async (it contains RELATES); previously the method-call arm
        // errored → fail-closed Deny for everyone. Now, with the RELATES side
        // false (NoOpResolver), the `contains` carries the result.
        let v = serde_json::json!({
            "auth": {"user_id": "u1", "groups": ["admin"]},
            "node": {"owner": "u2"}
        });
        let got = eval(
            "node.owner RELATES auth.user_id VIA 'guardian' || auth.groups.contains('admin')",
            v,
            &NoOpResolver,
        )
        .await;
        assert_eq!(got, Value::Boolean(true));
    }

    #[tokio::test]
    async fn method_on_left_of_and_with_relates() {
        // Method call evaluated async on the left of `&&`, RELATES on the right.
        let v = serde_json::json!({
            "auth": {"user_id": "u1", "groups": ["admin"]},
            "node": {"owner": "u2"}
        });
        let got = eval(
            "auth.groups.contains('admin') && node.owner RELATES auth.user_id VIA 'guardian'",
            v,
            &NoOpResolver,
        )
        .await;
        // contains == true, RELATES == false → false (no error).
        assert_eq!(got, Value::Boolean(false));
    }

    #[tokio::test]
    async fn chained_methods_evaluate_async() {
        // Method chaining inside an async tree (sibling RELATES forces async).
        let v = serde_json::json!({
            "auth": {"user_id": "u1", "name": "  Bob  "},
            "node": {"owner": "u2"}
        });
        let got = eval(
            "auth.name.trim().toLowerCase() == 'bob' || node.owner RELATES auth.user_id VIA 'g'",
            v,
            &NoOpResolver,
        )
        .await;
        assert_eq!(got, Value::Boolean(true));
    }

    #[tokio::test]
    async fn null_safe_receiver_in_async_tree() {
        // A method on a missing (null) receiver returns null, not an error,
        // even on the async path.
        let v = serde_json::json!({
            "auth": {"user_id": "u1"},
            "node": {"owner": "u2"}
        });
        let got = eval(
            "auth.missing.contains('x') || node.owner RELATES auth.user_id VIA 'g'",
            v,
            &NoOpResolver,
        )
        .await;
        // null (not truthy) OR false → false.
        assert_eq!(got, Value::Boolean(false));
    }

    #[tokio::test]
    async fn relates_true_short_circuits_method() {
        // RELATES true on the left of `||` must short-circuit so the method on
        // the right is never evaluated.
        let v = serde_json::json!({
            "auth": {"user_id": "u1", "groups": []},
            "node": {"owner": "u2"}
        });
        let got = eval(
            "node.owner RELATES auth.user_id VIA 'g' || auth.groups.contains('admin')",
            v,
            &FixedResolver(true),
        )
        .await;
        assert_eq!(got, Value::Boolean(true));
    }
}
