//! Decision step handler
//!
//! Evaluates raisin-rel conditions and branches accordingly.
//!
//! # Example
//!
//! ```yaml
//! nodes:
//!   - id: check-value
//!     type: decision
//!     properties:
//!       condition: "input.value > 10"
//!       yes_branch: "process-high"
//!       no_branch: "process-low"
//! ```

use super::{StepHandler, StepResult};
use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult};
use async_trait::async_trait;
use serde_json::Value;
use tracing::{debug, error, instrument};

/// Handler for decision steps
///
/// Evaluates a raisin-rel expression and branches based on the result.
/// The condition should evaluate to a boolean value.
#[derive(Debug)]
pub struct DecisionHandler;

impl DecisionHandler {
    /// Create a new decision handler
    pub fn new() -> Self {
        Self
    }

    /// Evaluate a condition expression
    #[instrument(skip(context))]
    fn evaluate_condition(&self, condition: &str, context: &FlowContext) -> FlowResult<bool> {
        debug!("Evaluating condition: {}", condition);

        // Build evaluation context from flow context
        // Convert variables to JSON for raisin-rel
        let mut eval_map = serde_json::Map::new();
        eval_map.insert("input".to_string(), context.input.clone());

        // Add all variables
        for (key, value) in &context.variables {
            eval_map.insert(key.clone(), value.clone());
        }

        let json_value = Value::Object(eval_map);

        // Convert to raisin-rel EvalContext
        let eval_ctx = raisin_rel::EvalContext::from_json(json_value).map_err(|e| {
            FlowError::ConditionEvaluation(format!("Invalid evaluation context: {}", e))
        })?;

        // Evaluate the expression
        let result = raisin_rel::eval(condition, &eval_ctx)
            .map_err(|e| FlowError::ConditionEvaluation(format!("Evaluation failed: {}", e)))?;

        // Convert result to boolean
        let bool_result = match result {
            raisin_rel::Value::Boolean(b) => b,
            raisin_rel::Value::Null => false,
            raisin_rel::Value::Integer(n) => n != 0,
            raisin_rel::Value::Float(f) => f != 0.0,
            raisin_rel::Value::String(s) => !s.is_empty(),
            raisin_rel::Value::Array(arr) => !arr.is_empty(),
            raisin_rel::Value::Object(obj) => !obj.is_empty(),
        };

        debug!("Condition evaluated to: {}", bool_result);
        Ok(bool_result)
    }

    /// Get the next node based on condition result
    fn get_next_node(&self, step: &FlowNode, condition_result: bool) -> FlowResult<String> {
        let branch_key = if condition_result {
            "yes_branch"
        } else {
            "no_branch"
        };

        step.get_string_property(branch_key).ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Decision node '{}' missing required property: {}",
                step.id, branch_key
            ))
        })
    }
}

impl Default for DecisionHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StepHandler for DecisionHandler {
    #[instrument(skip(self, context, _callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        _callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing decision step: {}", step.id);

        // Get the condition expression
        let condition = step.get_string_property("condition").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Decision node '{}' missing required property: condition",
                step.id
            ))
        })?;

        // Evaluate the condition
        let result = match self.evaluate_condition(&condition, context) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to evaluate condition: {}", e);
                return Err(e);
            }
        };

        // Determine next node
        let next_node_id = self.get_next_node(step, result)?;

        debug!("Decision result: {}, next node: {}", result, next_node_id);

        // Store decision result in context for debugging/audit
        context.set_variable(format!("{}_result", step.id), Value::Bool(result));

        Ok(StepResult::Continue {
            next_node_id,
            output: serde_json::json!({
                "decision": result,
                "branch_taken": if result { "yes" } else { "no" }
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StepType;
    use std::collections::HashMap;

    fn create_test_context() -> FlowContext {
        FlowContext::new(
            "test-instance".to_string(),
            serde_json::json!({
                "value": 42,
                "status": "active"
            }),
        )
    }

    fn create_decision_node() -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert(
            "condition".to_string(),
            Value::String("input.value > 10".to_string()),
        );
        properties.insert(
            "yes_branch".to_string(),
            Value::String("high-value".to_string()),
        );
        properties.insert(
            "no_branch".to_string(),
            Value::String("low-value".to_string()),
        );

        FlowNode {
            id: "decision-1".to_string(),
            step_type: StepType::Decision,
            properties,
            children: vec![],
            next_node: None,
        }
    }

    #[tokio::test]
    async fn test_decision_evaluates_true() {
        let handler = DecisionHandler::new();
        let context = create_test_context();

        let result = handler.evaluate_condition("input.value > 10", &context);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[tokio::test]
    async fn test_decision_evaluates_false() {
        let handler = DecisionHandler::new();
        let context = create_test_context();

        let result = handler.evaluate_condition("input.value < 10", &context);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn test_decision_complex_condition() {
        let handler = DecisionHandler::new();
        let context = FlowContext::new(
            "test-instance".to_string(),
            serde_json::json!({
                "priority": 7,
                "urgent": false,
                "enabled": true
            }),
        );

        let result = handler.evaluate_condition(
            "(input.priority >= 5 || input.urgent == true) && input.enabled == true",
            &context,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }
}
