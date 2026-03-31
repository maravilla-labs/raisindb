// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB

//! StepHandler execution for AI containers

use super::ai_call;
use super::types::{
    AiContainerState, AiMessage, MessageRole, ToolCall, ToolProcessingResult, ToolResult,
};
use super::AiContainerHandler;
use crate::handlers::conversation_persistence;
use crate::handlers::StepHandler;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowNode, FlowResult, StepResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, error, instrument, warn};

#[async_trait]
impl StepHandler for AiContainerHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing AI container step: {}", step.id);
        let step_start = Instant::now();

        // Emit step started event
        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_started(&step.id, None, "ai_container"),
            )
            .await;

        // Get configuration
        let config = self.get_config(step)?;

        // Resolve agent reference to get workspace and path
        let agent_ref = if config.agent_ref == "$auto" {
            // Derive from conversation's agent_ref property
            self.resolve_auto_agent_ref(context, callbacks).await?
        } else {
            // Parse the agent_ref string - could be a path or JSON reference
            self.parse_agent_ref(&config.agent_ref, step)?
        };

        debug!(
            "AI container config: agent_ref={}:{}, tool_mode={:?}, max_iterations={}",
            agent_ref.workspace, agent_ref.path, config.tool_mode, config.max_iterations
        );

        // Get current state
        let mut state = self.get_state(context, &step.id);

        // On first iteration, ensure conversation node exists and init user message
        if state.iteration == 0 {
            if let Ok(path) = self.get_conversation_path(context) {
                if let Err(e) = conversation_persistence::ensure_conversation(
                    callbacks,
                    &context.instance_id,
                    &path,
                    conversation_persistence::SYSTEM_WORKSPACE,
                    conversation_persistence::ConversationType::AiChat,
                    Some(config.agent_ref.as_str()),
                    &[],
                    None,
                )
                .await
                {
                    warn!("Failed to ensure conversation node: {}", e);
                }
            }
        }

        // Load conversation history from the node tree on every execution.
        // This avoids storing unbounded message arrays in flow variables.
        let mut messages: Vec<AiMessage> = Vec::new();
        if let Ok(path) = self.get_conversation_path(context) {
            match self.load_conversation_history(&path, callbacks).await {
                Ok(history) if !history.is_empty() => {
                    debug!("Loaded {} messages from conversation history", history.len());
                    messages = history;
                }
                Ok(_) => {
                    debug!("No prior conversation history found");
                }
                Err(e) => {
                    debug!("Could not load conversation history: {}, starting fresh", e);
                }
            }
        }

        // On first iteration, append the triggering user message if not already in history
        if state.iteration == 0 && messages.is_empty() {
            self.init_user_message_to(context, &mut messages);
            debug!(
                "Initialized conversation with {} messages",
                messages.len()
            );
        }

        // Check max iterations
        if state.iteration >= config.max_iterations {
            error!(
                "AI container exceeded max iterations: {}",
                config.max_iterations
            );
            return Err(FlowError::MaxIterationsExceeded {
                limit: config.max_iterations,
            });
        }

        // Record start time on first iteration (persisted across SameStep loops)
        if state.started_at_ms.is_none() {
            state.started_at_ms = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );
        }

        // Check total execution timeout
        if let (Some(started), Some(total_limit)) =
            (state.started_at_ms, config.total_timeout_ms)
        {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let elapsed = now_ms.saturating_sub(started);
            if elapsed > total_limit {
                error!(
                    "AI container total timeout exceeded: {}ms > {}ms",
                    elapsed, total_limit
                );
                return Err(FlowError::TimeoutExceeded {
                    duration_ms: total_limit,
                });
            }
        }

        // Increment iteration
        state.iteration += 1;
        debug!("AI container iteration: {}", state.iteration);

        // If we have pending tool results, add them to the loaded messages
        if self.has_pending_tool_results(&state) {
            debug!(
                "Processing {} pending tool results",
                state.tool_results.len()
            );

            for result in state.tool_results.drain(..) {
                let content = if let Some(error) = &result.error {
                    format!("Error: {}", error)
                } else {
                    serde_json::to_string(&result.result).unwrap_or_default()
                };

                messages.push(AiMessage {
                    role: MessageRole::Tool,
                    content,
                    tool_calls: None,
                    tool_call_id: Some(result.tool_call_id),
                });
            }
        }

        // If we have pending tool calls in explicit/hybrid mode, return wait state
        if !state.pending_tool_calls.is_empty() {
            debug!(
                "Waiting for {} explicit tool calls to complete",
                state.pending_tool_calls.len()
            );

            // Save state
            self.save_state(context, &step.id, &state)?;

            return Ok(StepResult::Wait {
                reason: "tool_call".to_string(),
                metadata: serde_json::json!({
                    "tool_calls": state.pending_tool_calls,
                    "iteration": state.iteration,
                    "step_id": step.id,
                }),
            });
        }

        // Check if agent is done
        if state.completed {
            debug!(
                "AI container completed after {} iterations",
                state.iteration
            );

            let output = serde_json::json!({
                "response": state.final_response,
                "iterations": state.iteration,
                "message_count": messages.len(),
            });

            // Emit step completed event
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

            // Get next node from flow
            let next_node_id = step
                .get_string_property("next_node")
                .unwrap_or_else(|| "end".to_string());

            return Ok(StepResult::Continue {
                next_node_id,
                output,
            });
        }

        // Call AI directly using the callback with timeout
        debug!(
            "Calling AI for agent: {}:{}",
            agent_ref.workspace, agent_ref.path
        );

        // Convert messages to JSON format for call_ai
        let messages_json: Vec<Value> = messages
            .iter()
            .map(|m| {
                let mut msg = serde_json::json!({
                    "role": match m.role {
                        MessageRole::System => "system",
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                        MessageRole::Tool => "tool",
                    },
                    "content": m.content,
                });
                if let Some(tool_calls) = &m.tool_calls {
                    msg["tool_calls"] = serde_json::to_value(tool_calls).unwrap_or_default();
                }
                if let Some(tool_call_id) = &m.tool_call_id {
                    msg["tool_call_id"] = Value::String(tool_call_id.clone());
                }
                msg
            })
            .collect();

        // Build response_format payload if configured
        let response_format = config.response_format.as_ref().map(|fmt| {
            let mut rf = serde_json::json!({ "type": fmt });
            if fmt == "json_schema" {
                if let Some(schema) = &config.output_schema {
                    rf["schema"] = schema.clone();
                }
            }
            rf
        });

        // AI call with streaming + retry on transient failures
        let ai_response = ai_call::call_ai_streaming_with_retry(
            callbacks,
            &agent_ref.workspace,
            &agent_ref.path,
            &messages_json,
            response_format,
            &context.instance_id,
            &config.execution,
        )
        .await?;

        debug!(
            "AI response received: {:?}",
            ai_response.get("finish_reason")
        );

        // Process AI response — text/thought chunks were already emitted during streaming
        let content = ai_response
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let finish_reason = ai_response
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("stop");

        // Check for tool calls
        let tool_calls: Vec<ToolCall> = ai_response
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        Some(ToolCall {
                            id: tc.get("id")?.as_str()?.to_string(),
                            name: tc.get("function")?.get("name")?.as_str()?.to_string(),
                            arguments: tc
                                .get("function")?
                                .get("arguments")
                                .map(|a| {
                                    if a.is_string() {
                                        serde_json::from_str(a.as_str().unwrap_or("{}"))
                                            .unwrap_or_default()
                                    } else {
                                        a.clone()
                                    }
                                })
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Process tool calls based on mode.
        // If no tool calls or stop reason, process_tool_calls returns NoTools.
        let processing_result = if tool_calls.is_empty() || finish_reason == "stop" {
            ToolProcessingResult::NoTools
        } else {
            self.process_tool_calls(&config, tool_calls)
        };

        match processing_result {
            ToolProcessingResult::NoTools => {
                state.completed = true;
                state.final_response = Some(content.clone());

                if let Err(e) = self
                    .save_assistant_response(context, callbacks, &content, &None, &ai_response)
                    .await
                {
                    error!("Failed to save assistant response: {}", e);
                }

                self.save_state(context, &step.id, &state)?;

                let output = serde_json::json!({
                    "response": state.final_response,
                    "iterations": state.iteration,
                    "message_count": messages.len(),
                });

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
            ToolProcessingResult::AutoExecute(tools) => {
                let message_path = self
                    .save_assistant_response(context, callbacks, &content, &Some(tools.clone()), &ai_response)
                    .await
                    .map_err(|e| { error!("Failed to save assistant response: {}", e); e })
                    .ok();

                debug!("Auto-executing {} tools", tools.len());
                execute_auto_tools(callbacks, &context.instance_id, &tools, message_path.as_deref(), &mut state).await;

                self.save_state(context, &step.id, &state)?;
                Ok(StepResult::SameStep {
                    metadata: serde_json::json!({ "tool_results_added": state.tool_results.len() }),
                })
            }
            ToolProcessingResult::ExplicitWait(tools) => {
                emit_tool_call_started_events(callbacks, &context.instance_id, &tools).await;

                state.pending_tool_calls = tools;
                self.save_state(context, &step.id, &state)?;

                Ok(StepResult::Wait {
                    reason: "tool_call".to_string(),
                    metadata: serde_json::json!({
                        "tool_calls": state.pending_tool_calls,
                        "iteration": state.iteration,
                        "step_id": step.id,
                    }),
                })
            }
            ToolProcessingResult::Mixed { auto_tools, explicit_tools } => {
                let all_tools: Vec<ToolCall> = auto_tools.iter().chain(explicit_tools.iter()).cloned().collect();
                let message_path = self
                    .save_assistant_response(context, callbacks, &content, &Some(all_tools), &ai_response)
                    .await
                    .map_err(|e| { error!("Failed to save assistant response: {}", e); e })
                    .ok();

                debug!("Mixed mode: auto-executing {} tools, waiting on {} explicit tools", auto_tools.len(), explicit_tools.len());
                execute_auto_tools(callbacks, &context.instance_id, &auto_tools, message_path.as_deref(), &mut state).await;

                emit_tool_call_started_events(callbacks, &context.instance_id, &explicit_tools).await;

                state.pending_tool_calls = explicit_tools;
                self.save_state(context, &step.id, &state)?;

                Ok(StepResult::Wait {
                    reason: "tool_call".to_string(),
                    metadata: serde_json::json!({
                        "tool_calls": state.pending_tool_calls,
                        "iteration": state.iteration,
                        "step_id": step.id,
                    }),
                })
            }
        }
    }
}

/// Execute auto-mode tools and collect results into state.
async fn execute_auto_tools(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    tools: &[ToolCall],
    message_path: Option<&str>,
    state: &mut AiContainerState,
) {
    for tool in tools {
        // Emit tool call started
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_started(
                    &tool.id,
                    &tool.name,
                    tool.arguments.clone(),
                ),
            )
            .await;

        // Resolve function path from tool map or use name directly
        let function_ref = message_path
            .map(|_| tool.name.clone())
            .unwrap_or_else(|| tool.name.clone());

        // Execute the tool
        let (result_value, error) = match callbacks
            .execute_function(&function_ref, tool.arguments.clone())
            .await
        {
            Ok(result) => (result, None),
            Err(e) => {
                error!("Tool execution failed for {}: {}", tool.name, e);
                (Value::Null, Some(e.to_string()))
            }
        };

        // Emit tool call completed
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_completed(&tool.id, result_value.clone(), error.clone(), None),
            )
            .await;

        state.tool_results.push(ToolResult {
            tool_call_id: tool.id.clone(),
            name: tool.name.clone(),
            result: result_value,
            error,
        });
    }
}

/// Emit ToolCallStarted events for explicit/wait-mode tools.
async fn emit_tool_call_started_events(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    tools: &[ToolCall],
) {
    for tool in tools {
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_started(
                    &tool.id,
                    &tool.name,
                    tool.arguments.clone(),
                ),
            )
            .await;
    }
}
