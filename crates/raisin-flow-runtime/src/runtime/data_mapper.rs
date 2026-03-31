// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Data mapping and expression evaluation utility.
//!
//! This module provides the `DataMapper` struct which is responsible for
//! resolving variable references and evaluating expressions within flow steps.
//! It supports:
//! - Recursive resolution in JSON Objects and Arrays
//! - Variable substitution (e.g., "${input.user.name}")
//! - Expression evaluation using `raisin-rel`

use crate::types::{FlowContext, FlowError, FlowResult};
use serde_json::Value;
use tracing::{debug, warn};

/// Utility for mapping data and evaluating expressions
pub struct DataMapper;

impl DataMapper {
    /// Map a value by resolving any expressions or variables within it.
    ///
    /// This method recursively processes the input value:
    /// - Strings matching "${...}" are evaluated as expressions
    /// - Objects and Arrays are processed recursively
    /// - Other values are returned as-is
    ///
    /// # Arguments
    /// * `value` - The value to map
    /// * `context` - The flow execution context
    ///
    /// # Returns
    /// The resolved value
    pub fn map(value: &Value, context: &FlowContext) -> FlowResult<Value> {
        match value {
            Value::String(s) => Self::resolve_string(s, context),
            Value::Array(arr) => {
                let mut resolved_arr = Vec::with_capacity(arr.len());
                for item in arr {
                    resolved_arr.push(Self::map(item, context)?);
                }
                Ok(Value::Array(resolved_arr))
            }
            Value::Object(obj) => {
                let mut resolved_obj = serde_json::Map::with_capacity(obj.len());
                for (k, v) in obj {
                    // We map values, but keys are kept as-is
                    resolved_obj.insert(k.clone(), Self::map(v, context)?);
                }
                Ok(Value::Object(resolved_obj))
            }
            _ => Ok(value.clone()),
        }
    }

    /// Resolve a string value, checking for expression markers.
    ///
    /// Supported formats:
    /// - "${expression}" - Evaluates the expression using raisin-rel
    /// - "{{variable}}" - Simple variable substitution (legacy support)
    fn resolve_string(s: &str, context: &FlowContext) -> FlowResult<Value> {
        let trimmed = s.trim();

        // Check for expression syntax: ${...}
        if trimmed.starts_with("${") && trimmed.ends_with('}') {
            let expr = &trimmed[2..trimmed.len() - 1];
            return Self::evaluate_expression(expr, context);
        }

        // Check for template syntax: {{...}}
        if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
            let expr = &trimmed[2..trimmed.len() - 2];
            return Self::evaluate_expression(expr, context);
        }

        // Return literal string
        Ok(Value::String(s.to_string()))
    }

    /// Evaluate a raisin-rel expression against the flow context.
    fn evaluate_expression(expr: &str, context: &FlowContext) -> FlowResult<Value> {
        debug!("Evaluating expression: {}", expr);

        // Build evaluation context from flow context
        let mut eval_map = serde_json::Map::new();

        // 1. Add "input" (the triggering node data)
        eval_map.insert("input".to_string(), context.input.clone());

        // 2. Add "steps" (outputs from previous steps)
        // This matches the proposed `${steps.step_id.output}` syntax
        if !context.step_outputs.is_empty() {
            let steps_val = serde_json::to_value(&context.step_outputs).unwrap_or(Value::Null);
            eval_map.insert("steps".to_string(), steps_val);
        }

        // 3. Add variables directly at root
        for (key, value) in &context.variables {
            eval_map.insert(key.clone(), value.clone());
        }

        // 4. Add trigger info
        if let Some(trigger) = &context.trigger_info {
            if let Ok(val) = serde_json::to_value(trigger) {
                eval_map.insert("trigger".to_string(), val);
            }
        }

        let json_context = Value::Object(eval_map);

        // Convert to raisin-rel EvalContext
        let eval_ctx = raisin_rel::EvalContext::from_json(json_context).map_err(|e| {
            FlowError::ConditionEvaluation(format!("Invalid evaluation context: {}", e))
        })?;

        // Evaluate
        let result = raisin_rel::eval(expr, &eval_ctx).map_err(|e| {
            FlowError::ConditionEvaluation(format!("Evaluation failed for '{}': {}", expr, e))
        })?;

        // Convert raisin-rel Value back to serde_json::Value
        Ok(match result {
            raisin_rel::Value::Null => Value::Null,
            raisin_rel::Value::Boolean(b) => Value::Bool(b),
            raisin_rel::Value::Integer(i) => Value::Number(i.into()),
            raisin_rel::Value::Float(f) => {
                if let Some(n) = serde_json::Number::from_f64(f) {
                    Value::Number(n)
                } else {
                    warn!("Expression resulted in non-finite float: {}", f);
                    Value::Null
                }
            }
            raisin_rel::Value::String(s) => Value::String(s),
            raisin_rel::Value::Array(arr) => {
                // Convert array items recursively (simplified)
                Value::Array(
                    arr.into_iter()
                        .map(|v| {
                            match v {
                                raisin_rel::Value::String(s) => Value::String(s),
                                raisin_rel::Value::Integer(i) => Value::Number(i.into()),
                                raisin_rel::Value::Boolean(b) => Value::Bool(b),
                                raisin_rel::Value::Null => Value::Null,
                                _ => Value::String(format!("{:?}", v)), // Fallback for complex types
                            }
                        })
                        .collect(),
                )
            }
            raisin_rel::Value::Object(obj) => {
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    map.insert(k, Value::String(format!("{:?}", v))); // Simplified
                }
                Value::Object(map)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_context() -> FlowContext {
        let mut context = FlowContext::new(
            "test-id".to_string(),
            json!({
                "user": {
                    "name": "Alice",
                    "age": 30
                }
            }),
        );
        context.set_variable("status".to_string(), Value::String("active".to_string()));
        context.record_step_output("step_1".to_string(), json!({"result": 100}));
        context
    }

    #[test]
    fn test_map_literal() {
        let context = create_test_context();
        let val = json!("hello");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!("hello"));
    }

    #[test]
    fn test_map_input_expression() {
        let context = create_test_context();
        let val = json!("${input.user.name}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!("Alice"));
    }

    #[test]
    fn test_map_steps_expression() {
        let context = create_test_context();
        let val = json!("${steps.step_1.result}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!(100));
    }

    #[test]
    fn test_map_math_expression() {
        let context = create_test_context();
        let val = json!("${input.user.age + 5}");
        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result, json!(35));
    }

    #[test]
    fn test_map_recursive_structure() {
        let context = create_test_context();
        let val = json!({
            "name": "${input.user.name}",
            "info": {
                "age_next_year": "${input.user.age + 1}"
            },
            "list": ["${status}", "literal"]
        });

        let result = DataMapper::map(&val, &context).unwrap();
        assert_eq!(result["name"], json!("Alice"));
        assert_eq!(result["info"]["age_next_year"], json!(31));
        assert_eq!(result["list"][0], json!("active"));
        assert_eq!(result["list"][1], json!("literal"));
    }
}
