//! Expression evaluation for Cypher
//!
//! Provides functions to evaluate Cypher expressions to values.
//! Supports literals, variables, properties, binary operations, and function calls.

use std::collections::HashMap;

use raisin_cypher_parser::{Expr, Literal};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use super::functions::{evaluate_function, FunctionContext};
use crate::physical_plan::cypher::types::VariableBinding;
use crate::physical_plan::executor::ExecutionError;

type BoxedFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Evaluate an expression to a value (synchronous wrapper)
///
/// This is a blocking wrapper around the async implementation.
/// Use this when calling from synchronous contexts within a tokio runtime.
///
/// # Arguments
///
/// * `expr` - Expression to evaluate
/// * `binding` - Current variable binding
/// * `context` - Function evaluation context
///
/// # Returns
///
/// Result containing the computed PropertyValue or an ExecutionError
pub fn evaluate_expr<S: Storage>(
    expr: &Expr,
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, ExecutionError> {
    // Use tokio's block_in_place to call async from sync context
    // This is safe because we're in a tokio runtime context
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(evaluate_expr_async_impl(expr, binding, context))
    })
}

/// Evaluate an expression asynchronously (returns a boxed future)
///
/// This wrapper returns a pinned boxed future for cases where the caller
/// needs to work with the future directly rather than awaiting immediately.
///
/// # Arguments
///
/// * `expr` - Expression to evaluate
/// * `binding` - Current variable binding
/// * `context` - Function evaluation context
///
/// # Returns
///
/// Pinned boxed future resolving to Result<PropertyValue, ExecutionError>
pub fn evaluate_expr_async<'a, S: Storage>(
    expr: &'a Expr,
    binding: &'a VariableBinding,
    context: &'a FunctionContext<'_, S>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<PropertyValue, ExecutionError>> + Send + 'a>,
> {
    Box::pin(async move { evaluate_expr_async_impl(expr, binding, context).await })
}

/// Implementation of async expression evaluation
///
/// This is the core evaluation logic. It handles:
/// - Literals (strings, numbers, booleans, null)
/// - Variables (node/relationship references)
/// - Property access (node.id, node.property)
/// - Binary operations (+, -, *, /, comparisons, logical ops)
/// - Function calls (delegated to function registry)
///
/// # Arguments
///
/// * `expr` - Expression to evaluate
/// * `binding` - Current variable binding with matched nodes/relationships
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Result containing the computed PropertyValue or an ExecutionError
pub fn evaluate_expr_async_impl<'a, S: Storage>(
    expr: &'a Expr,
    binding: &'a VariableBinding,
    context: &'a FunctionContext<'_, S>,
) -> BoxedFuture<'a, Result<PropertyValue, ExecutionError>> {
    Box::pin(async move {
        match expr {
            // Literal values
            Expr::Literal(lit) => match lit {
                Literal::String(s) => Ok(PropertyValue::String(s.clone())),
                Literal::Integer(i) => Ok(PropertyValue::Integer(*i)),
                Literal::Float(f) => Ok(PropertyValue::Float(*f)),
                Literal::Boolean(b) => Ok(PropertyValue::Boolean(*b)),
                Literal::Null => {
                    // PropertyValue doesn't have a Null variant
                    // Use empty Object as placeholder
                    Ok(PropertyValue::Object(HashMap::new()))
                }
            },

            // Property access: node.property or relationship.property
            Expr::Property { expr, property } => {
                // Evaluate expr to get variable name
                if let Expr::Variable(var) = &**expr {
                    // Try nodes first
                    if let Some(node) = binding.get_node(var) {
                        // Support both built-in properties and custom properties
                        let value = match property.as_str() {
                            "id" => PropertyValue::String(node.id.clone()),
                            "workspace" => PropertyValue::String(node.workspace.clone()),
                            "path" => PropertyValue::String(node.path.clone()),
                            "type" => PropertyValue::String(node.node_type.clone()),
                            // Custom properties from properties HashMap
                            _ => node.properties.get(property).cloned().unwrap_or_else(|| {
                                tracing::warn!(
                                    "   Property '{}' not found on node, returning empty string",
                                    property
                                );
                                PropertyValue::String(String::new())
                            }),
                        };
                        return Ok(value);
                    }
                    // Try relationships
                    else if let Some(rel_info) = binding.relationships.get(var) {
                        let value =
                            match property.as_str() {
                                "type" => PropertyValue::String(rel_info.relation_type.clone()),
                                "source" => PropertyValue::String(rel_info.source_var.clone()),
                                "target" => PropertyValue::String(rel_info.target_var.clone()),
                                // Check relationship properties (e.g., weight)
                                _ => rel_info.properties.get(property).cloned().unwrap_or_else(
                                    || {
                                        tracing::debug!(
                                    "   Property '{}' not found on relationship, returning null",
                                    property
                                );
                                        // Return null as empty object (PropertyValue has no Null variant)
                                        PropertyValue::Object(HashMap::new())
                                    },
                                ),
                            };
                        return Ok(value);
                    } else {
                        return Err(ExecutionError::Validation(format!(
                            "Variable '{}' not found in binding",
                            var
                        )));
                    }
                }
                Err(ExecutionError::Validation(
                    "Property access only supported on variables (e.g., node.id)".to_string(),
                ))
            }

            // Parameter reference: $param
            Expr::Parameter(name) => {
                if let Some(value) = context.parameters.get(name) {
                    Ok(value.clone())
                } else {
                    Err(ExecutionError::Validation(format!(
                        "Parameter '${}' not provided",
                        name
                    )))
                }
            }

            // Variable reference: return whole node or relationship as Object
            Expr::Variable(var) => {
                if let Some(node_info) = binding.get_node(var) {
                    // Convert NodeInfo to PropertyValue (as an Object)
                    let mut node_map = HashMap::new();
                    node_map.insert(
                        "id".to_string(),
                        PropertyValue::String(node_info.id.clone()),
                    );
                    node_map.insert(
                        "workspace".to_string(),
                        PropertyValue::String(node_info.workspace.clone()),
                    );
                    node_map.insert(
                        "path".to_string(),
                        PropertyValue::String(node_info.path.clone()),
                    );
                    node_map.insert(
                        "type".to_string(),
                        PropertyValue::String(node_info.node_type.clone()),
                    );
                    node_map.insert(
                        "properties".to_string(),
                        PropertyValue::Object(node_info.properties.clone()),
                    );
                    Ok(PropertyValue::Object(node_map))
                } else if let Some(rel_info) = binding.relationships.get(var) {
                    // Convert RelationInfo to PropertyValue (as an Object)
                    let mut rel_map = HashMap::new();
                    rel_map.insert(
                        "type".to_string(),
                        PropertyValue::String(rel_info.relation_type.clone()),
                    );
                    rel_map.insert(
                        "source_var".to_string(),
                        PropertyValue::String(rel_info.source_var.clone()),
                    );
                    rel_map.insert(
                        "target_var".to_string(),
                        PropertyValue::String(rel_info.target_var.clone()),
                    );
                    Ok(PropertyValue::Object(rel_map))
                } else {
                    Err(ExecutionError::Validation(format!(
                        "Variable '{}' not found in binding",
                        var
                    )))
                }
            }

            // Function calls: delegate to function registry
            Expr::FunctionCall { name, args, .. } => {
                evaluate_function(name, args, binding, context).await
            }

            // Binary operations: +, -, *, /, =, !=, <, >, <=, >=, AND, OR
            Expr::BinaryOp { left, op, right } => {
                use raisin_cypher_parser::BinOp;

                let left_val = evaluate_expr_async_impl(left, binding, context).await?;
                let right_val = evaluate_expr_async_impl(right, binding, context).await?;

                match op {
                    // Arithmetic operations - return Float since division always returns float
                    BinOp::Add => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Float(l as f64 + r as f64))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Float(l as f64 + r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Float(l + r as f64))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Float(l + r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Addition requires numeric operands".to_string(),
                        )),
                    },
                    BinOp::Sub => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Float(l as f64 - r as f64))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Float(l as f64 - r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Float(l - r as f64))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Float(l - r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Subtraction requires numeric operands".to_string(),
                        )),
                    },
                    BinOp::Mul => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Float(l as f64 * r as f64))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Float(l as f64 * r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Float(l * r as f64))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Float(l * r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Multiplication requires numeric operands".to_string(),
                        )),
                    },
                    BinOp::Div => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            if r == 0 {
                                Err(ExecutionError::Validation("Division by zero".to_string()))
                            } else {
                                Ok(PropertyValue::Float(l as f64 / r as f64))
                            }
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            if r == 0.0 {
                                Err(ExecutionError::Validation("Division by zero".to_string()))
                            } else {
                                Ok(PropertyValue::Float(l as f64 / r))
                            }
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            if r == 0 {
                                Err(ExecutionError::Validation("Division by zero".to_string()))
                            } else {
                                Ok(PropertyValue::Float(l / r as f64))
                            }
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            if r == 0.0 {
                                Err(ExecutionError::Validation("Division by zero".to_string()))
                            } else {
                                Ok(PropertyValue::Float(l / r))
                            }
                        }
                        _ => Err(ExecutionError::Validation(
                            "Division requires numeric operands".to_string(),
                        )),
                    },

                    // Comparison operations - handle both Integer and Float
                    BinOp::Eq => Ok(PropertyValue::Boolean(left_val == right_val)),
                    BinOp::Neq => Ok(PropertyValue::Boolean(left_val != right_val)),
                    BinOp::Lt => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l < r))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean((l as f64) < r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l < (r as f64)))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean(l < r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Comparison requires numeric operands".to_string(),
                        )),
                    },
                    BinOp::Lte => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l <= r))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean((l as f64) <= r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l <= (r as f64)))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean(l <= r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Comparison requires numeric operands".to_string(),
                        )),
                    },
                    BinOp::Gt => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l > r))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean((l as f64) > r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l > (r as f64)))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean(l > r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Comparison requires numeric operands".to_string(),
                        )),
                    },
                    BinOp::Gte => match (left_val, right_val) {
                        (PropertyValue::Integer(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l >= r))
                        }
                        (PropertyValue::Integer(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean((l as f64) >= r))
                        }
                        (PropertyValue::Float(l), PropertyValue::Integer(r)) => {
                            Ok(PropertyValue::Boolean(l >= (r as f64)))
                        }
                        (PropertyValue::Float(l), PropertyValue::Float(r)) => {
                            Ok(PropertyValue::Boolean(l >= r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "Comparison requires numeric operands".to_string(),
                        )),
                    },

                    // Logical operations
                    BinOp::And => match (left_val, right_val) {
                        (PropertyValue::Boolean(l), PropertyValue::Boolean(r)) => {
                            Ok(PropertyValue::Boolean(l && r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "AND requires boolean operands".to_string(),
                        )),
                    },
                    BinOp::Or => match (left_val, right_val) {
                        (PropertyValue::Boolean(l), PropertyValue::Boolean(r)) => {
                            Ok(PropertyValue::Boolean(l || r))
                        }
                        _ => Err(ExecutionError::Validation(
                            "OR requires boolean operands".to_string(),
                        )),
                    },

                    _ => Err(ExecutionError::Validation(format!(
                        "Unsupported binary operator: {:?}",
                        op
                    ))),
                }
            }

            // List construction: [expr1, expr2, expr3, ...]
            Expr::List(items) => {
                let mut list = Vec::new();
                for item_expr in items {
                    let item_value = evaluate_expr_async_impl(item_expr, binding, context).await?;
                    list.push(item_value);
                }
                Ok(PropertyValue::Array(list))
            }

            _ => Err(ExecutionError::Validation(format!(
                "Unsupported expression: {:?}",
                expr
            ))),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::cypher::types::VariableBinding;
    use raisin_storage_memory::InMemoryStorage;
    use std::collections::HashMap;

    fn function_context<'a>(
        storage: &'a InMemoryStorage,
        parameters: &'a HashMap<String, PropertyValue>,
    ) -> FunctionContext<'a, InMemoryStorage> {
        FunctionContext {
            storage,
            tenant_id: "tenant",
            repo_id: "repo",
            branch: "main",
            workspace_id: "workspace",
            revision: None,
            parameters,
        }
    }

    #[tokio::test]
    async fn parameter_expression_returns_bound_value() {
        let storage = InMemoryStorage::default();
        let mut params = HashMap::new();
        params.insert("limit".to_string(), PropertyValue::Integer(5));
        let ctx = function_context(&storage, &params);
        let expr = Expr::Parameter("limit".into());
        let binding = VariableBinding::new();

        let value = evaluate_expr_async_impl(&expr, &binding, &ctx)
            .await
            .expect("parameter resolves");

        assert_eq!(value, PropertyValue::Integer(5));
    }

    #[tokio::test]
    async fn missing_parameter_returns_error() {
        let storage = InMemoryStorage::default();
        let params = HashMap::new();
        let ctx = function_context(&storage, &params);
        let expr = Expr::Parameter("unknown".into());
        let binding = VariableBinding::new();

        let err = evaluate_expr_async_impl(&expr, &binding, &ctx)
            .await
            .expect_err("missing param should error");

        let ExecutionError::Validation(message) = err else {
            panic!("expected validation error");
        };
        assert!(message.contains("$unknown"));
    }
}
