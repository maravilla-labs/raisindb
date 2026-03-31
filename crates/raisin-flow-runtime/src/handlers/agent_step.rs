// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Lightweight single-shot AI agent step handler.
//!
//! Calls an agent once with the flow context as input. No tool loop,
//! no conversation persistence, no mode overrides. Uses the agent's
//! configuration as-is.
//!
//! Use cases: classify email, extract entities, sentiment analysis,
//! generate summary.

use super::{StepHandler, StepResult};
use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowNode, FlowResult};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, error, instrument, warn};

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

        // Build user message from flow context input
        let user_content = context
            .input
            .get("event")
            .and_then(|e| e.get("node_data"))
            .and_then(|n| n.get("properties"))
            .and_then(|p| p.get("content"))
            .and_then(|c| c.as_str())
            .or_else(|| context.input.get("message").and_then(|v| v.as_str()))
            .or_else(|| context.input.get("input").and_then(|v| v.as_str()))
            .unwrap_or("");

        let messages = vec![serde_json::json!({
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

        let ai_response = callbacks
            .call_ai(&agent_workspace, &agent_ref, messages, response_format.clone())
            .await
            .map_err(|e| {
                error!("Agent step AI call failed: {}", e);
                FlowError::AIProvider(format!("Agent step AI call failed: {}", e))
            })?;

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
            .get_string_property("next_node")
            .unwrap_or_else(|| "end".to_string());

        Ok(StepResult::Continue {
            next_node_id,
            output,
        })
    }
}
