// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB

//! StepHandler execution for AI containers

use super::ai_call;
use super::skill_grant;
use super::types::{
    AiContainerState, AiMessage, MessageRole, ToolCall, ToolProcessingResult, ToolResult,
};
use super::AiContainerHandler;
use crate::handlers::ai_tool_loop;
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
                    debug!(
                        "Loaded {} messages from conversation history",
                        history.len()
                    );
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
            self.init_user_message_to(step, context, &mut messages);
            debug!("Initialized conversation with {} messages", messages.len());
            // PERSIST IT. History is reloaded from the conversation node on
            // every re-entry (after a tool result, after a park), and a
            // history that starts with the assistant's tool call and no user
            // turn is what made the model answer "please provide the title
            // and description you'd like evaluated" on its second call.
            if let (Some(first), Ok(path)) = (messages.first(), self.get_conversation_path(context))
            {
                if first.role == MessageRole::User && !first.content.is_empty() {
                    if let Err(e) = conversation_persistence::persist_user_message(
                        callbacks,
                        &context.instance_id,
                        &path,
                        conversation_persistence::SYSTEM_WORKSPACE,
                        &first.content,
                        Some("flow"),
                        Some("Automation"),
                    )
                    .await
                    {
                        warn!("Failed to persist the initial user message: {}", e);
                    }
                }
            }
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
        if let (Some(started), Some(total_limit)) = (state.started_at_ms, config.total_timeout_ms) {
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

        // If we have pending tool calls in explicit/hybrid mode, return wait
        // state. Checked BEFORE the results are drained: a drain here would
        // move them into this call's local transcript only, and the saved
        // state would lose them — a refused call's result included, leaving
        // the assistant turn with a tool call nothing ever answers.
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
                    name: Some(result.name),
                });
            }
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
                .next_node
                .clone()
                .or_else(|| step.get_string_property("next_node"))
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
                if let Some(name) = &m.name {
                    msg["name"] = Value::String(name.clone());
                }
                msg
            })
            .collect();

        // Build response_format payload if configured — in the shape
        // `raisin_ai::types::ResponseFormat` deserializes (`json_schema:
        // {name, schema, strict}`), the same one agent_step sends. The old
        // `{type, schema}` was refused by `from_value` and silently dropped.
        let response_format = config.response_format.as_ref().map(|fmt| {
            let mut rf = serde_json::json!({ "type": fmt });
            if fmt == "json_schema" {
                if let Some(schema) = &config.output_schema {
                    rf["json_schema"] = serde_json::json!({
                        "name": "container_output",
                        "schema": schema.clone(),
                        "strict": false
                    });
                }
            }
            rf
        });

        // AI call with streaming + retry on transient failures. The step's own
        // `skills:` go with it, added to the agent's for this step only.
        let step_skills = crate::handlers::ai_tool_loop::step_skills(step);
        let ai_response = ai_call::call_ai_streaming_with_retry(
            callbacks,
            &agent_ref.workspace,
            &agent_ref.path,
            &messages_json,
            response_format,
            &step_skills,
            &context.instance_id,
            &config.execution,
        )
        .await?;

        debug!(
            "AI response received: {:?}",
            ai_response.get("finish_reason")
        );

        // Tool NAME → FUNCTION PATH, from the callback (via `_tool_map` on the
        // accumulated response). Auto-executed tools are resolved through it;
        // executing by bare name asked for a function called `update-node`.
        // It is also the whole OFFER: a name outside it is refused, never run.
        let tool_paths = crate::handlers::ai_tool_loop::tool_map_of(&ai_response);

        // The SKILL GRANT the callback resolved for this agent and step. It is
        // what `load-skill` is executed with; nothing the model writes is.
        let skill_grant = skill_grant::from_response(&ai_response);

        // Accumulate this turn's usage onto the container's running total, so a
        // tool loop's cost survives the SameStep re-entries and reaches the
        // flow's metrics in the final output.
        if let Some((input, output)) = crate::runtime::executor::extract_token_usage(&ai_response) {
            state.input_tokens = state.input_tokens.saturating_add(input);
            state.output_tokens = state.output_tokens.saturating_add(output);
        }

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
                        let arguments = tc
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
                            .unwrap_or_default();
                        Some(ToolCall {
                            id: tc.get("id")?.as_str()?.to_string(),
                            name: tc.get("function")?.get("name")?.as_str()?.to_string(),
                            // The MODEL wrote these, so any runtime-only key in
                            // them is a forgery: gone before the call is stored,
                            // shown, parked for an explicit executor or run.
                            arguments: skill_grant::strip_runtime_keys(arguments),
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
                    "usage": {
                        "input_tokens": state.input_tokens,
                        "output_tokens": state.output_tokens,
                    },
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
                    .next_node
                    .clone()
                    .or_else(|| step.get_string_property("next_node"))
                    .unwrap_or_else(|| "end".to_string());

                Ok(StepResult::Continue {
                    next_node_id,
                    output,
                })
            }
            ToolProcessingResult::AutoExecute(tools) => {
                let message_path = self
                    .save_assistant_response(
                        context,
                        callbacks,
                        &content,
                        &Some(tools.clone()),
                        &ai_response,
                    )
                    .await
                    .map_err(|e| {
                        error!("Failed to save assistant response: {}", e);
                        e
                    })
                    .ok();

                debug!("Auto-executing {} tools", tools.len());
                execute_auto_tools(
                    callbacks,
                    &context.instance_id,
                    &agent_ref.path,
                    &tools,
                    &tool_paths,
                    skill_grant.as_ref(),
                    &mut state,
                )
                .await;

                self.save_state(context, &step.id, &state)?;
                Ok(StepResult::SameStep {
                    metadata: serde_json::json!({ "tool_results_added": state.tool_results.len() }),
                })
            }
            ToolProcessingResult::ExplicitWait(tools) => {
                // An explicit executor runs what is parked, so an unoffered
                // call is refused HERE, before it can be handed on.
                let called = tools.clone();
                let tools = refuse_unoffered(
                    callbacks,
                    &context.instance_id,
                    tools,
                    &tool_paths,
                    &mut state,
                )
                .await;
                if tools.is_empty() {
                    // Everything was refused: nothing to wait for. Keep the
                    // assistant turn (its tool results need it) and let the
                    // model read the refusals on the next iteration.
                    if let Err(e) = self
                        .save_assistant_response(
                            context,
                            callbacks,
                            &content,
                            &Some(called),
                            &ai_response,
                        )
                        .await
                    {
                        error!("Failed to save assistant response: {}", e);
                    }
                    self.save_state(context, &step.id, &state)?;
                    return Ok(StepResult::SameStep {
                        metadata: serde_json::json!({ "tool_results_added": state.tool_results.len() }),
                    });
                }
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
            ToolProcessingResult::Mixed {
                auto_tools,
                explicit_tools,
            } => {
                let all_tools: Vec<ToolCall> = auto_tools
                    .iter()
                    .chain(explicit_tools.iter())
                    .cloned()
                    .collect();
                let message_path = self
                    .save_assistant_response(
                        context,
                        callbacks,
                        &content,
                        &Some(all_tools),
                        &ai_response,
                    )
                    .await
                    .map_err(|e| {
                        error!("Failed to save assistant response: {}", e);
                        e
                    })
                    .ok();

                debug!(
                    "Mixed mode: auto-executing {} tools, waiting on {} explicit tools",
                    auto_tools.len(),
                    explicit_tools.len()
                );
                execute_auto_tools(
                    callbacks,
                    &context.instance_id,
                    &agent_ref.path,
                    &auto_tools,
                    &tool_paths,
                    skill_grant.as_ref(),
                    &mut state,
                )
                .await;

                let explicit_tools = refuse_unoffered(
                    callbacks,
                    &context.instance_id,
                    explicit_tools,
                    &tool_paths,
                    &mut state,
                )
                .await;
                if explicit_tools.is_empty() {
                    self.save_state(context, &step.id, &state)?;
                    return Ok(StepResult::SameStep {
                        metadata: serde_json::json!({ "tool_results_added": state.tool_results.len() }),
                    });
                }
                emit_tool_call_started_events(callbacks, &context.instance_id, &explicit_tools)
                    .await;

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

/// Record a refusal for every call naming a tool the model was not OFFERED,
/// and return the calls that were. A refusal is a tool result carrying an
/// error — the model reads it next iteration — never a crash.
async fn refuse_unoffered(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    tools: Vec<ToolCall>,
    tool_paths: &std::collections::HashMap<String, String>,
    state: &mut AiContainerState,
) -> Vec<ToolCall> {
    let mut offered = Vec::with_capacity(tools.len());
    for tool in tools {
        if ai_tool_loop::offered_function(&tool.name, tool_paths).is_some() {
            offered.push(tool);
            continue;
        }
        let message = ai_tool_loop::unoffered_tool_error(&tool.name, tool_paths);
        warn!(tool = %tool.name, "Model called a tool it was not offered - refusing");
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_started(&tool.id, &tool.name, tool.arguments.clone()),
            )
            .await;
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_completed(
                    &tool.id,
                    Value::Null,
                    Some(message.clone()),
                    Some(0),
                ),
            )
            .await;
        state.tool_results.push(ToolResult {
            tool_call_id: tool.id.clone(),
            name: tool.name.clone(),
            result: Value::Null,
            error: Some(message),
        });
    }
    offered
}

/// Execute auto-mode tools and collect results into state.
///
/// `skill_grant` is the callback's `_skill_grant` for this turn; a
/// `load-skill` call is executed with it and with nothing the model supplied.
/// Each tool runs AS the agent (`agent_path`), so the agent's own configured
/// rights apply rather than the flow's; a name outside `tool_paths` — the
/// offer — is refused, not run.
async fn execute_auto_tools(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    agent_path: &str,
    tools: &[ToolCall],
    tool_paths: &std::collections::HashMap<String, String>,
    skill_grant: Option<&Value>,
    state: &mut AiContainerState,
) {
    let tools = refuse_unoffered(callbacks, instance_id, tools.to_vec(), tool_paths, state).await;
    for tool in &tools {
        // Emit tool call started
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_started(&tool.id, &tool.name, tool.arguments.clone()),
            )
            .await;

        // Offered, so it has a path (refuse_unoffered kept only those).
        let Some(function_ref) =
            ai_tool_loop::offered_function(&tool.name, tool_paths).map(String::from)
        else {
            continue;
        };

        // Execute the tool — with the arguments the RUNTIME settles on.
        let arguments = skill_grant::tool_arguments(
            &tool.name,
            &function_ref,
            tool.arguments.clone(),
            skill_grant,
        );
        let (result_value, error) = match callbacks
            .execute_function_as_agent(&function_ref, arguments, agent_path)
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
                FlowExecutionEvent::tool_call_completed(
                    &tool.id,
                    result_value.clone(),
                    error.clone(),
                    None,
                ),
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
                FlowExecutionEvent::tool_call_started(&tool.id, &tool.name, tool.arguments.clone()),
            )
            .await;
    }
}

#[cfg(test)]
mod offered_tool_tests {
    use super::*;
    use crate::handlers::ai_tool_loop::test_support::{turn, ScriptedCallbacks};
    use crate::types::StepType;
    use serde_json::json;
    use std::collections::HashMap;

    fn step(tool_mode: &str) -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert("agent_ref".to_string(), json!("/agents/builder"));
        properties.insert("tool_mode".to_string(), json!(tool_mode));
        FlowNode {
            id: "build".to_string(),
            step_type: StepType::AIContainer,
            properties,
            children: vec![],
            next_node: Some("end".to_string()),
        }
    }

    fn offer() -> Option<Value> {
        Some(json!({ "lookup": "/lib/x/lookup" }))
    }

    async fn run(callbacks: &ScriptedCallbacks, node: &FlowNode) -> (StepResult, AiContainerState) {
        let mut context =
            FlowContext::new("i-1".to_string(), json!({ "user_message": "Build it." }));
        let handler = AiContainerHandler::new();
        let result = handler
            .execute(node, &mut context, callbacks)
            .await
            .expect("container step runs");
        let state = handler.get_state(&context, &node.id);
        (result, state)
    }

    /// The security gap: `unwrap_or_else(|| tool.name.clone())` ran ANY
    /// function the model named, with the flow's authority.
    #[tokio::test]
    async fn a_model_naming_an_unoffered_function_is_refused() {
        let callbacks = ScriptedCallbacks::new(vec![turn(
            &[(
                "c1",
                "/lib/studio/builder/arm-generated-function",
                json!({ "function_path": "/x" }),
            )],
            offer(),
        )]);
        let (result, state) = run(&callbacks, &step("auto")).await;

        assert!(matches!(result, StepResult::SameStep { .. }), "{result:?}");
        assert!(
            callbacks.executed().is_empty(),
            "{:?}",
            callbacks.executed()
        );
        let refusal = &state.tool_results[0];
        assert_eq!(refusal.tool_call_id, "c1");
        let error = refusal
            .error
            .as_deref()
            .expect("a tool error the model reads");
        assert!(error.contains("is not a tool offered to you"), "{error}");
        assert!(error.contains("lookup"), "{error}");
    }

    /// Mixed batch: the offered call still runs — as the AGENT, not the flow —
    /// and only the unoffered one is refused.
    #[tokio::test]
    async fn an_offered_call_runs_as_the_agent_beside_a_refused_one() {
        let callbacks = ScriptedCallbacks::new(vec![turn(
            &[
                ("c1", "lookup", json!({ "q": "x" })),
                ("c2", "delete-everything", json!({})),
            ],
            offer(),
        )]);
        let (_, state) = run(&callbacks, &step("auto")).await;

        assert_eq!(
            callbacks.executed(),
            vec![(
                "/lib/x/lookup".to_string(),
                json!({ "q": "x" }),
                Some("/agents/builder".to_string())
            )]
        );
        let refused: Vec<_> = state
            .tool_results
            .iter()
            .filter(|r| r.error.is_some())
            .collect();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].name, "delete-everything");
    }

    /// Explicit mode hands parked calls to another executor, so an unoffered
    /// one is refused before it is parked; with nothing left there is nothing
    /// to wait for.
    #[tokio::test]
    async fn an_unoffered_call_is_never_parked_for_an_explicit_executor() {
        let callbacks =
            ScriptedCallbacks::new(vec![turn(&[("c1", "/lib/secret", json!({}))], offer())]);
        let (result, state) = run(&callbacks, &step("explicit")).await;

        assert!(matches!(result, StepResult::SameStep { .. }), "{result:?}");
        assert!(state.pending_tool_calls.is_empty());
        assert!(state.tool_results[0].error.is_some());
        assert!(callbacks.executed().is_empty());
    }

    #[tokio::test]
    async fn an_offered_explicit_call_is_still_parked() {
        let callbacks = ScriptedCallbacks::new(vec![turn(
            &[
                ("c1", "lookup", json!({})),
                ("c2", "/lib/secret", json!({})),
            ],
            offer(),
        )]);
        let (result, state) = run(&callbacks, &step("explicit")).await;

        let StepResult::Wait { metadata, .. } = result else {
            panic!("expected Wait, got {result:?}");
        };
        assert_eq!(metadata["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(metadata["tool_calls"][0]["name"], "lookup");
        assert_eq!(
            state.tool_results.len(),
            1,
            "the refusal is kept for the model"
        );
    }

    /// Re-entering while the offered call is still parked must not drop the
    /// refusal: it answers a tool call the saved assistant turn carries.
    #[tokio::test]
    async fn a_refusal_survives_a_re_entry_while_a_call_is_parked() {
        let callbacks = ScriptedCallbacks::new(vec![turn(
            &[
                ("c1", "lookup", json!({})),
                ("c2", "/lib/secret", json!({})),
            ],
            offer(),
        )]);
        let node = step("explicit");
        let mut context =
            FlowContext::new("i-1".to_string(), json!({ "user_message": "Build it." }));
        let handler = AiContainerHandler::new();
        handler
            .execute(&node, &mut context, &callbacks)
            .await
            .expect("first entry");
        let again = handler
            .execute(&node, &mut context, &callbacks)
            .await
            .expect("re-entry");

        assert!(matches!(again, StepResult::Wait { .. }), "{again:?}");
        let state = handler.get_state(&context, &node.id);
        assert_eq!(state.pending_tool_calls.len(), 1);
        assert_eq!(state.tool_results.len(), 1, "the refusal is still there");
        assert!(callbacks.executed().is_empty());
    }
}
