// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Sub-flow step handler for calling nested workflows.
//!
//! This handler allows a workflow to invoke another workflow as a step,
//! enabling composition and reuse of workflow logic.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult};

use super::StepHandler;

/// Handler for sub-flow steps that invoke nested workflows.
///
/// Sub-flows enable workflow composition:
/// - A parent flow can call a child flow as a single step
/// - Input is mapped from parent context to child input
/// - Child output is merged back into parent context
/// - Errors propagate up with proper context
pub struct SubFlowHandler;

impl SubFlowHandler {
    /// Create a new sub-flow handler
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubFlowHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StepHandler for SubFlowHandler {
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        // Get sub-flow reference from properties
        let flow_ref = step
            .get_string_property("flow_ref")
            .or_else(|| step.get_string_property("flow_path"))
            .ok_or_else(|| {
                FlowError::MissingProperty(
                    "flow_ref or flow_path required for sub-flow step".to_string(),
                )
            })?;

        tracing::info!(
            step_id = %step.id,
            flow_ref = %flow_ref,
            "Executing sub-flow step"
        );

        // Build input for child flow
        let child_input = build_child_input(step, context)?;

        // Get execution mode: sync or async
        let async_mode = step
            .properties
            .get("async")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if async_mode {
            // Queue child flow as job and wait
            let job_payload = json!({
                "flow_path": flow_ref,
                "input": child_input,
                "parent_instance_id": context.instance_id,
                "parent_step_id": step.id,
            });

            let job_id = callbacks.queue_job("flow_execution", job_payload).await?;

            tracing::info!(
                step_id = %step.id,
                job_id = %job_id,
                "Sub-flow queued for async execution"
            );

            // Wait for child flow to complete
            Ok(StepResult::Wait {
                reason: "sub_flow".to_string(),
                metadata: json!({
                    "job_id": job_id,
                    "flow_ref": flow_ref,
                    "wait_type": "sub_flow_completion",
                }),
            })
        } else {
            // Sync execution - load and execute child flow inline
            // This is a simplified version - full implementation would use the executor

            // For now, queue as job but return immediately
            // The job handler will handle the actual execution
            let job_payload = json!({
                "flow_path": flow_ref,
                "input": child_input,
                "parent_instance_id": context.instance_id,
                "parent_step_id": step.id,
                "sync_mode": true,
            });

            let job_id = callbacks.queue_job("flow_execution", job_payload).await?;

            // Wait for completion
            Ok(StepResult::Wait {
                reason: "sub_flow".to_string(),
                metadata: json!({
                    "job_id": job_id,
                    "flow_ref": flow_ref,
                    "wait_type": "sub_flow_completion",
                }),
            })
        }
    }
}

/// Build input object for child flow from step properties and context
fn build_child_input(step: &FlowNode, context: &FlowContext) -> FlowResult<Value> {
    // Check for explicit input mapping
    if let Some(input_mapping) = step.properties.get("input_mapping") {
        if let Some(mapping_obj) = input_mapping.as_object() {
            let mut child_input = serde_json::Map::new();

            for (target_key, source_expr) in mapping_obj {
                // Simple variable reference (e.g., "$variable_name" or "$.path.to.value")
                if let Some(expr_str) = source_expr.as_str() {
                    if expr_str.starts_with("$.") || expr_str.starts_with("$") {
                        // Variable reference - extract from context
                        let var_name = expr_str.trim_start_matches("$.").trim_start_matches('$');
                        if let Some(value) = context.variables.get(var_name) {
                            child_input.insert(target_key.clone(), value.clone());
                        }
                    } else {
                        // Literal string value
                        child_input.insert(target_key.clone(), source_expr.clone());
                    }
                } else {
                    // Direct value
                    child_input.insert(target_key.clone(), source_expr.clone());
                }
            }

            return Ok(Value::Object(child_input));
        }
    }

    // Check for direct input property
    if let Some(input) = step.properties.get("input") {
        return Ok(input.clone());
    }

    // Default: pass parent's input and variables
    let mut child_input = serde_json::Map::new();

    // Include parent input
    if let Some(parent_input) = context.input.as_object() {
        for (k, v) in parent_input {
            child_input.insert(k.clone(), v.clone());
        }
    }

    // Include context variables
    for (k, v) in &context.variables {
        child_input.insert(k.clone(), v.clone());
    }

    Ok(Value::Object(child_input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_context() -> FlowContext {
        let mut variables = HashMap::new();
        variables.insert("user_id".to_string(), json!("user123"));
        variables.insert("amount".to_string(), json!(100));

        FlowContext {
            instance_id: "test-instance".to_string(),
            trigger_info: None,
            input: json!({"request_id": "req-123"}),
            step_outputs: HashMap::new(),
            variables,
            current_output: None,
            error: None,
            context_stack: Vec::new(),
            compensation_stack: Vec::new(),
        }
    }

    #[test]
    fn test_build_child_input_with_mapping() {
        let context = create_test_context();

        let mut properties = HashMap::new();
        properties.insert(
            "input_mapping".to_string(),
            json!({
                "userId": "$user_id",
                "transactionAmount": "$amount",
                "static_value": "hello"
            }),
        );

        let step = FlowNode {
            id: "sub-flow-1".to_string(),
            step_type: crate::types::StepType::SubFlow,
            properties,
            children: vec![],
            next_node: None,
        };

        let result = build_child_input(&step, &context).unwrap();
        let obj = result.as_object().unwrap();

        assert_eq!(obj.get("userId"), Some(&json!("user123")));
        assert_eq!(obj.get("transactionAmount"), Some(&json!(100)));
        assert_eq!(obj.get("static_value"), Some(&json!("hello")));
    }

    #[test]
    fn test_build_child_input_default() {
        let context = create_test_context();

        let step = FlowNode {
            id: "sub-flow-1".to_string(),
            step_type: crate::types::StepType::SubFlow,
            properties: HashMap::new(),
            children: vec![],
            next_node: None,
        };

        let result = build_child_input(&step, &context).unwrap();
        let obj = result.as_object().unwrap();

        // Should include both input and variables
        assert_eq!(obj.get("request_id"), Some(&json!("req-123")));
        assert_eq!(obj.get("user_id"), Some(&json!("user123")));
        assert_eq!(obj.get("amount"), Some(&json!(100)));
    }
}
