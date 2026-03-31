// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step execution helpers for the flow execution handler
//!
//! Provides instance loading/saving, context building, and step dispatch logic.

use std::collections::HashMap;

use crate::handlers::{
    AiContainerHandler, ChatStepHandler, DecisionHandler, FunctionStepHandler, HumanTaskHandler,
    ParallelHandler, StepHandler,
};
use crate::types::{
    FlowCallbacks, FlowContext, FlowDefinition, FlowError, FlowInstance, FlowNode, FlowResult,
    StepResult, StepType, WaitType,
};

use super::handler::FlowExecutionHandler;

impl FlowExecutionHandler {
    /// Load a flow instance by ID
    ///
    /// The instance path follows the pattern: `/flows/instances/{instance_id}`
    pub(super) async fn load_instance(&self, instance_id: &str) -> FlowResult<FlowInstance> {
        let instance_path = format!("/flows/instances/{}", instance_id);

        self.callbacks
            .load_instance(&instance_path)
            .await
            .map_err(|e| {
                FlowError::NodeNotFound(format!("Flow instance '{}' not found: {}", instance_id, e))
            })
    }

    /// Save flow instance state
    ///
    /// This uses optimistic concurrency control to prevent concurrent updates
    pub(super) async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
        self.callbacks
            .save_instance(instance)
            .await
            .map_err(|e| FlowError::Other(format!("Failed to save flow instance: {}", e)))
    }

    /// Build flow context from instance
    pub(super) fn build_context(&self, instance: &FlowInstance) -> FlowResult<FlowContext> {
        // Parse step outputs from instance variables
        let step_outputs: HashMap<String, serde_json::Value> =
            if let Some(obj) = instance.variables.as_object() {
                if let Some(steps_obj) = obj.get("step_outputs").and_then(|v| v.as_object()) {
                    steps_obj
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                } else {
                    HashMap::new()
                }
            } else {
                HashMap::new()
            };

        // Parse regular variables (excluding step_outputs)
        let mut variables: HashMap<String, serde_json::Value> =
            if let Some(obj) = instance.variables.as_object() {
                obj.iter()
                    .filter(|(k, _)| k.as_str() != "step_outputs")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else {
                HashMap::new()
            };

        // Add compensation stack to variables
        variables.insert(
            "_compensation_stack".to_string(),
            serde_json::to_value(&instance.compensation_stack).unwrap_or(serde_json::Value::Null),
        );

        Ok(FlowContext {
            instance_id: instance.id.clone(),
            trigger_info: None, // TODO: Restore from instance if needed
            input: instance.input.clone(),
            step_outputs,
            variables,
            current_output: None,
            error: None,
            context_stack: vec![],
            compensation_stack: instance.compensation_stack.clone(),
        })
    }

    /// Update instance from context after step execution
    pub(super) fn update_instance_from_context(
        &self,
        instance: &mut FlowInstance,
        context: &FlowContext,
    ) {
        // Build variables object with step_outputs
        let mut vars_map = serde_json::Map::new();

        // Add step outputs
        if !context.step_outputs.is_empty() {
            vars_map.insert(
                "step_outputs".to_string(),
                serde_json::to_value(&context.step_outputs).unwrap_or(serde_json::Value::Null),
            );
        }

        // Add regular variables (excluding internal ones)
        for (key, value) in &context.variables {
            if !key.starts_with('_') {
                vars_map.insert(key.clone(), value.clone());
            }
        }

        instance.variables = serde_json::Value::Object(vars_map);
        instance.compensation_stack = context.compensation_stack.clone();
    }

    /// Execute the current step
    pub(super) async fn execute_current_step(
        &self,
        instance: &mut FlowInstance,
        flow_definition: &FlowDefinition,
        context: &mut FlowContext,
    ) -> FlowResult<StepResult> {
        // Find current node in flow definition
        let current_node = flow_definition
            .find_node(&instance.current_node_id)
            .ok_or_else(|| {
                FlowError::StepNotFound(format!(
                    "Node '{}' not found in flow definition",
                    instance.current_node_id
                ))
            })?;

        tracing::debug!(
            flow_instance_id = %instance.id,
            node_id = %current_node.id,
            step_type = ?current_node.step_type,
            "Executing flow step"
        );

        // Increment step count
        instance.metrics.step_count += 1;

        // Dispatch to appropriate handler based on step type
        let result = self.dispatch_step(current_node, context).await?;

        Ok(result)
    }

    /// Dispatch step execution to the appropriate handler
    async fn dispatch_step(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
    ) -> FlowResult<StepResult> {
        match &step.step_type {
            StepType::Start => {
                // Start node - continue to next node
                let next_node_id = step
                    .next_node
                    .clone()
                    .or_else(|| step.get_string_property("next_node"))
                    .ok_or_else(|| {
                        FlowError::MissingProperty(format!(
                            "Start node '{}' missing next_node",
                            step.id
                        ))
                    })?;

                Ok(StepResult::Continue {
                    next_node_id,
                    output: serde_json::json!({
                        "started": true
                    }),
                })
            }
            StepType::End => {
                // End node - complete the flow
                let output = context.build_step_input();
                Ok(StepResult::Complete { output })
            }
            StepType::Decision => {
                let handler = DecisionHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::FunctionStep => {
                let handler = FunctionStepHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::AgentStep => {
                use crate::handlers::AgentStepHandler;
                let handler = AgentStepHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::AIContainer => {
                let handler = AiContainerHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::HumanTask => {
                let handler = HumanTaskHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::Parallel => {
                let handler = ParallelHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::Join => Err(FlowError::Other(
                "Join nodes not yet implemented".to_string(),
            )),
            StepType::Wait => Err(FlowError::Other(
                "Wait nodes not yet implemented".to_string(),
            )),
            StepType::SubFlow => Err(FlowError::Other(
                "Sub-flow nodes not yet implemented".to_string(),
            )),
            StepType::Container => Err(FlowError::Other(
                "Container nodes not yet implemented".to_string(),
            )),
            StepType::Loop => Err(FlowError::Other(
                "Loop nodes not yet implemented".to_string(),
            )),
            StepType::Chat => {
                let handler = ChatStepHandler::new();
                handler
                    .execute(step, context, self.callbacks.as_ref())
                    .await
            }
            StepType::Custom(custom_type) => Err(FlowError::Other(format!(
                "Custom step type '{}' not supported",
                custom_type
            ))),
        }
    }

    /// Determine wait type from reason string
    pub(super) fn determine_wait_type(&self, reason: &str) -> WaitType {
        match reason {
            "function_call" => WaitType::ToolCall,
            "ai_tool_call" => WaitType::ToolCall,
            "human_task" => WaitType::HumanTask,
            "scheduled" => WaitType::Scheduled,
            "retry" => WaitType::Retry,
            "join" => WaitType::Join,
            "chat_session" => WaitType::ChatSession,
            _ => WaitType::Event,
        }
    }
}
