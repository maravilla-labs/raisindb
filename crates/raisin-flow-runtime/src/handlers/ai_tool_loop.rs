// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reusable AI tool call loop for chat and container handlers.
//!
//! When an AI model responds with `tool_calls`, this module:
//! 1. Executes each tool via `FlowCallbacks::execute_function`
//! 2. Feeds results back to the AI as `role: tool` messages
//! 3. Repeats until the AI responds without tool calls or the limit is hit
//! 4. Collects all executed tool calls for persistence / event emission

use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowResult};
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, warn};

/// Maximum tool-loop iterations to prevent runaway.
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 10;

/// Configuration for an AI-with-tools invocation.
pub struct ToolLoopConfig {
    pub agent_workspace: String,
    pub agent_path: String,
    pub max_tool_iterations: u32,
    pub response_format: Option<Value>,
}

impl ToolLoopConfig {
    pub fn new(agent_workspace: &str, agent_path: &str) -> Self {
        Self {
            agent_workspace: agent_workspace.to_string(),
            agent_path: agent_path.to_string(),
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            response_format: None,
        }
    }
}

/// Result of a full AI invocation (possibly spanning multiple tool-call rounds).
pub struct ToolLoopResult {
    /// Final text content from the AI
    pub content: String,
    /// All tool calls that were executed during the loop
    pub tool_calls_executed: Vec<ExecutedToolCall>,
    /// Model that produced the response
    pub model: Option<String>,
    /// Finish reason from the final response
    pub finish_reason: Option<String>,
    /// Aggregated token usage across all rounds
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    /// Whether the AI signaled end-of-session
    pub end_session: bool,
    /// Optional handoff target
    pub handoff_to: Option<String>,
}

/// A single tool call that was executed.
pub struct ExecutedToolCall {
    pub id: String,
    pub function_name: String,
    pub arguments: Value,
    pub result: Value,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Call an AI agent and automatically handle tool-call loops.
///
/// Returns the final response after all tool calls have been resolved.
pub async fn run_ai_with_tools(
    callbacks: &dyn FlowCallbacks,
    mut messages: Vec<Value>,
    config: &ToolLoopConfig,
    instance_id: &str,
) -> FlowResult<ToolLoopResult> {
    let mut all_tool_calls: Vec<ExecutedToolCall> = Vec::new();
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut last_model: Option<String> = None;
    // Tool name → function path mapping, extracted from the first AI response.
    let mut tool_map: std::collections::HashMap<String, String> = Default::default();

    for iteration in 0..=config.max_tool_iterations {
        let response = callbacks
            .call_ai(
                &config.agent_workspace,
                &config.agent_path,
                messages.clone(),
                config.response_format.clone(),
            )
            .await?;

        // Extract tool name → path mapping (included by ai_callback)
        if tool_map.is_empty() {
            if let Some(map) = response.get("_tool_map") {
                if let Ok(parsed) =
                    serde_json::from_value::<std::collections::HashMap<String, String>>(map.clone())
                {
                    tool_map = parsed;
                }
            }
        }

        // Accumulate usage
        if let Some(usage) = response.get("usage") {
            total_input_tokens += usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            total_output_tokens += usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }

        last_model = response
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Check for tool calls
        let tool_calls = response
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if tool_calls.is_empty() || iteration == config.max_tool_iterations {
            // No tool calls or iteration limit — return final response
            if iteration == config.max_tool_iterations && !tool_calls.is_empty() {
                warn!(
                    "Tool loop hit max iterations ({}), returning partial response",
                    config.max_tool_iterations
                );
            }

            let content = response
                .get("content")
                .or_else(|| response.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            return Ok(ToolLoopResult {
                content,
                tool_calls_executed: all_tool_calls,
                model: last_model,
                finish_reason: response
                    .get("finish_reason")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                total_input_tokens,
                total_output_tokens,
                end_session: response
                    .get("end_session")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                handoff_to: response
                    .get("handoff_to")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }

        // Build assistant message with tool_calls for the conversation history
        let assistant_content = response
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
            "tool_calls": tool_calls,
        }));

        // Execute each tool call
        for tc in &tool_calls {
            let executed =
                execute_tool_call(callbacks, tc, &tool_map, instance_id, iteration).await;
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": executed.id,
                "content": serde_json::to_string(&executed.result).unwrap_or_default(),
            }));
            all_tool_calls.push(executed);
        }
    }

    // Should not reach here, but just in case
    Ok(ToolLoopResult {
        content: String::new(),
        tool_calls_executed: all_tool_calls,
        model: last_model,
        finish_reason: None,
        total_input_tokens,
        total_output_tokens,
        end_session: false,
        handoff_to: None,
    })
}

/// Call an AI agent with streaming, emitting text/thought chunks in real time.
///
/// Falls back to the non-streaming `call_ai` path (via the trait default) but
/// always consumes the response through the channel API so callers get the same
/// `ToolLoopResult` regardless of whether the provider truly streams.
pub async fn run_ai_with_tools_streaming(
    callbacks: &dyn FlowCallbacks,
    mut messages: Vec<Value>,
    config: &ToolLoopConfig,
    instance_id: &str,
) -> FlowResult<ToolLoopResult> {
    let mut all_tool_calls: Vec<ExecutedToolCall> = Vec::new();
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut last_model: Option<String> = None;
    let mut tool_map: std::collections::HashMap<String, String> = Default::default();

    for iteration in 0..=config.max_tool_iterations {
        let mut rx = callbacks
            .call_ai_streaming(
                &config.agent_workspace,
                &config.agent_path,
                messages.clone(),
                config.response_format.clone(),
            )
            .await?;

        // Accumulate the full response from streaming chunks
        let response = accumulate_stream(&mut rx, callbacks, instance_id, &mut tool_map).await;

        // Accumulate usage
        if let Some(usage) = response.get("usage") {
            total_input_tokens += usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            total_output_tokens += usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }

        last_model = response
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from);

        let tool_calls = response
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if tool_calls.is_empty() || iteration == config.max_tool_iterations {
            if iteration == config.max_tool_iterations && !tool_calls.is_empty() {
                warn!(
                    "Streaming tool loop hit max iterations ({})",
                    config.max_tool_iterations
                );
            }
            let content = response
                .get("content")
                .or_else(|| response.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            return Ok(ToolLoopResult {
                content,
                tool_calls_executed: all_tool_calls,
                model: last_model,
                finish_reason: response
                    .get("finish_reason")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                total_input_tokens,
                total_output_tokens,
                end_session: response
                    .get("end_session")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                handoff_to: response
                    .get("handoff_to")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }

        // Tool calls present — execute them (same logic as non-streaming)
        let assistant_content = response
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
            "tool_calls": tool_calls,
        }));

        for tc in &tool_calls {
            let executed =
                execute_tool_call(callbacks, tc, &tool_map, instance_id, iteration).await;
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": executed.id,
                "content": serde_json::to_string(&executed.result).unwrap_or_default(),
            }));
            all_tool_calls.push(executed);
        }
    }

    Ok(ToolLoopResult {
        content: String::new(),
        tool_calls_executed: all_tool_calls,
        model: last_model,
        finish_reason: None,
        total_input_tokens,
        total_output_tokens,
        end_session: false,
        handoff_to: None,
    })
}

/// Read all chunks from a streaming AI channel, emitting events and building the final response.
///
/// Delegates to [`raisin_ai::streaming::accumulate_stream`] with a callback that
/// emits `text_chunk` and `thought_chunk` flow execution events in real-time.
pub async fn accumulate_stream(
    rx: &mut tokio::sync::mpsc::Receiver<Value>,
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    tool_map: &mut std::collections::HashMap<String, String>,
) -> Value {
    use raisin_ai::streaming::StreamEvent;

    raisin_ai::streaming::accumulate_stream(
        rx,
        |event| async move {
            match event {
                StreamEvent::TextChunk(text) => {
                    let _ = callbacks
                        .emit_event(instance_id, FlowExecutionEvent::text_chunk(&text))
                        .await;
                }
                StreamEvent::ThoughtChunk(text) => {
                    let _ = callbacks
                        .emit_event(instance_id, FlowExecutionEvent::thought_chunk(&text))
                        .await;
                }
            }
        },
        tool_map,
    )
    .await
}

/// Merge a streaming tool call delta into the accumulated tool_calls array (public alias).
pub fn merge_streaming_tool_call_pub(tool_calls: &mut Vec<Value>, delta: &Value) {
    raisin_ai::streaming::merge_streaming_tool_call(tool_calls, delta);
}

/// Execute a single tool call, emitting start/complete events.
async fn execute_tool_call(
    callbacks: &dyn FlowCallbacks,
    tc: &Value,
    tool_map: &std::collections::HashMap<String, String>,
    instance_id: &str,
    iteration: u32,
) -> ExecutedToolCall {
    let tc_id = tc
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let func = tc.get("function").cloned().unwrap_or_default();
    let func_name = func
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let arguments = parse_tool_arguments(&func);

    debug!(tool_call_id = %tc_id, function = %func_name, "Executing tool call (iteration {})", iteration);

    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::tool_call_started(&tc_id, &func_name, arguments.clone()),
        )
        .await;

    let func_path = tool_map
        .get(&func_name)
        .cloned()
        .unwrap_or_else(|| func_name.clone());

    let start = Instant::now();
    let exec_result = callbacks
        .execute_function(&func_path, arguments.clone())
        .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    let (result_value, error) = match exec_result {
        Ok(val) => (val, None),
        Err(e) => {
            warn!("Tool call {} ({}) failed: {}", tc_id, func_name, e);
            (
                serde_json::json!({"error": e.to_string()}),
                Some(e.to_string()),
            )
        }
    };

    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::tool_call_completed(
                &tc_id,
                result_value.clone(),
                error.clone(),
                Some(duration_ms),
            ),
        )
        .await;

    ExecutedToolCall {
        id: tc_id,
        function_name: func_name,
        arguments,
        result: result_value,
        error,
        duration_ms,
    }
}

/// Parse tool arguments from the function object.
///
/// The `arguments` field may be a JSON string (OpenAI format) or already a Value.
fn parse_tool_arguments(func: &Value) -> Value {
    match func.get("arguments") {
        Some(Value::String(s)) => {
            serde_json::from_str(s).unwrap_or(Value::Object(Default::default()))
        }
        Some(v) => v.clone(),
        None => Value::Object(Default::default()),
    }
}
