// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Lightweight single-shot AI agent step handler.
//!
//! Calls an agent with the flow context as input. The agent's OWN tools
//! (configured on the agent node) are executed in a bounded internal
//! loop — synchronously, with no conversation persistence and no child
//! steps. For workflow-level tools, orchestration, or explicit tool
//! visibility, use an `ai_sequence` container instead.
//!
//! Use cases: classify email, extract entities, sentiment analysis,
//! generate summary — including agents that need a lookup tool to do so.

use super::{StepHandler, StepResult};
use crate::runtime::DataMapper;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowNode, FlowResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, error, instrument, warn};

/// Default cap for the internal tool loop of an agent step
const MAX_TOOL_ITERATIONS_DEFAULT: u32 = 5;

/// Handler for single-shot AI agent steps.
#[derive(Debug)]
pub struct AgentStepHandler;

impl AgentStepHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgentStepHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StepHandler for AgentStepHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing agent step: {}", step.id);
        let step_start = Instant::now();

        // Emit step started event
        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_started(&step.id, None, "agent_step"),
            )
            .await;

        // Get agent reference
        let agent_ref = step
            .get_string_property("agent_ref")
            .or_else(|| {
                step.get_property("agent_ref")
                    .and_then(|v| v.as_object())
                    .and_then(|obj| {
                        obj.get("raisin:path")
                            .or_else(|| obj.get("raisin:ref"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
            })
            .ok_or_else(|| {
                FlowError::MissingProperty(format!(
                    "Agent step '{}' missing required property: agent_ref",
                    step.id
                ))
            })?;

        let agent_workspace = step
            .get_string_property("agent_workspace")
            .unwrap_or_else(|| "functions".to_string());

        // Build user message: an explicit `prompt` (or `message`) property is
        // template-resolved against the flow context (e.g.
        // "Summarize: {{ steps.fetch.body }}"); otherwise fall back to
        // digging the triggering content out of the flow input.
        let user_content: String = match step
            .get_property("prompt")
            .or_else(|| step.get_property("message"))
        {
            Some(prompt_value) => match DataMapper::map(prompt_value, context)? {
                Value::String(s) => s,
                Value::Null => String::new(),
                other => other.to_string(),
            },
            None => context
                .input
                .get("event")
                .and_then(|e| e.get("node_data"))
                .and_then(|n| n.get("properties"))
                .and_then(|p| p.get("content"))
                .and_then(|c| c.as_str())
                .or_else(|| context.input.get("message").and_then(|v| v.as_str()))
                .or_else(|| context.input.get("input").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string(),
        };

        // Optional workflow-context injection (include_context property:
        // "input" | "full" | true) - templates stay the precise mechanism
        let user_content = super::context_injection::with_context_block(
            user_content,
            context,
            super::context_injection::ContextInjection::from_step(step),
        );

        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": user_content,
        })];

        debug!(
            "Calling agent: {}:{} with {} chars of input",
            agent_workspace,
            agent_ref,
            user_content.len()
        );

        // Read optional response_format from step properties for structured output
        let response_format = step.get_property("response_format").cloned();

        if response_format.is_some() {
            debug!("Agent step '{}' has response_format configured", step.id);
        }

        // Bounded internal tool loop: the agent's own tools (advertised by
        // call_ai from the agent node config) are executed here so a
        // tool-equipped agent works in a single-shot step. Workflow-level
        // tools / explicit tool steps remain ai_sequence territory.
        let max_tool_iterations = step
            .get_u32_property("max_tool_iterations")
            .unwrap_or(MAX_TOOL_ITERATIONS_DEFAULT);
        let mut tool_iterations: u32 = 0;
        let mut tools_used: Vec<Value> = Vec::new();

        let ai_response = loop {
            let response = callbacks
                .call_ai(
                    &agent_workspace,
                    &agent_ref,
                    messages.clone(),
                    response_format.clone(),
                )
                .await
                .map_err(|e| {
                    error!("Agent step AI call failed: {}", e);
                    FlowError::AIProvider(format!("Agent step AI call failed: {}", e))
                })?;

            let tool_calls: Vec<Value> = response
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if tool_calls.is_empty() {
                break response;
            }
            if tool_iterations >= max_tool_iterations {
                warn!(
                    "Agent step '{}' hit max_tool_iterations ({}) with tool calls still pending - using last response",
                    step.id, max_tool_iterations
                );
                break response;
            }
            tool_iterations += 1;

            // tool name -> function path mapping provided by call_ai
            let tool_map = response.get("_tool_map").cloned().unwrap_or(Value::Null);

            // Echo the assistant turn (with its tool calls) into the transcript
            messages.push(serde_json::json!({
                "role": "assistant",
                "content": response.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "tool_calls": tool_calls,
            }));

            for call in &tool_calls {
                let call_id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments: Value = call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .map(|a| {
                        if let Some(s) = a.as_str() {
                            serde_json::from_str(s).unwrap_or(Value::Null)
                        } else {
                            a.clone()
                        }
                    })
                    .unwrap_or(Value::Null);
                let function_ref = tool_map
                    .get(name)
                    .and_then(|v| v.as_str())
                    .unwrap_or(name)
                    .to_string();

                let _ = callbacks
                    .emit_event(
                        &context.instance_id,
                        FlowExecutionEvent::tool_call_started(call_id, name, arguments.clone()),
                    )
                    .await;

                let (result, tool_error) = match callbacks
                    .execute_function(&function_ref, arguments.clone())
                    .await
                {
                    Ok(result) => (result, None),
                    Err(e) => {
                        warn!(
                            "Agent step '{}' tool '{}' failed: {} - feeding error back to agent",
                            step.id, name, e
                        );
                        (Value::Null, Some(e.to_string()))
                    }
                };

                let _ = callbacks
                    .emit_event(
                        &context.instance_id,
                        FlowExecutionEvent::tool_call_completed(
                            call_id,
                            result.clone(),
                            tool_error.clone(),
                            None,
                        ),
                    )
                    .await;

                let tool_content = match &tool_error {
                    Some(err) => format!("Error: {}", err),
                    None => match &result {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                };
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": tool_content,
                }));
                tools_used.push(serde_json::json!({
                    "name": name,
                    "function_ref": function_ref,
                    "error": tool_error,
                }));
            }
        };

        let content = ai_response
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Emit text chunk
        if !content.is_empty() {
            let _ = callbacks
                .emit_event(
                    &context.instance_id,
                    FlowExecutionEvent::text_chunk(&content),
                )
                .await;
        }

        // If response_format was requested, try to parse the content as JSON
        let structured_output = if response_format.is_some() && !content.is_empty() {
            match serde_json::from_str::<Value>(&content) {
                Ok(parsed) => {
                    debug!("Parsed structured output from agent step '{}'", step.id);
                    Some(parsed)
                }
                Err(e) => {
                    warn!(
                        "Agent step '{}': response_format set but content is not valid JSON: {}",
                        step.id, e
                    );
                    None
                }
            }
        } else {
            None
        };

        let mut output = serde_json::json!({
            "response": content,
            "model": ai_response.get("model"),
            "finish_reason": ai_response.get("finish_reason"),
            "usage": ai_response.get("usage"),
        });

        if let Some(data) = structured_output {
            output["structured_output"] = data;
        }
        if !tools_used.is_empty() {
            output["tools_used"] = Value::Array(tools_used);
            output["tool_iterations"] = serde_json::json!(tool_iterations);
        }

        // Emit step completed
        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_completed(
                    &step.id,
                    output.clone(),
                    step_start.elapsed().as_millis() as u64,
                ),
            )
            .await;

        let next_node_id = step
            .next_node
            .clone()
            .or_else(|| step.get_string_property("next_node"))
            .unwrap_or_else(|| "end".to_string());

        Ok(StepResult::Continue {
            next_node_id,
            output,
        })
    }
}
