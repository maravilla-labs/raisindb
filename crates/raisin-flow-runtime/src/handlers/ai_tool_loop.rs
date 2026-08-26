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
//!
//! # Control tools
//!
//! Not every tool call is work to execute. [`control_tools`] defines verbs the
//! RUNTIME implements — end the session with a result, hand off, ask a human —
//! and this loop intercepts those BY NAME before `execute_function` is reached.
//! Interception is why a control tool must never be routed onward: an
//! unresolved tool name falls back to being called as a function PATH, so a
//! missed interception would try to invoke `/end_session`.
//!
//! A control call ends the loop rather than feeding a result back, because all
//! three verbs are about what happens NEXT: two of them finish the turn and the
//! third parks the flow mid-turn, to be resumed with the human's answer standing
//! in as that tool's result (see [`PendingApproval`]).

use crate::handlers::control_tools::{self, ApprovalRequest, ControlAction, ControlToolConfig};
use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowResult};
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Maximum tool-loop iterations to prevent runaway.
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 10;

/// Configuration for an AI-with-tools invocation.
pub struct ToolLoopConfig {
    pub agent_workspace: String,
    pub agent_path: String,
    pub max_tool_iterations: u32,
    pub response_format: Option<Value>,
    /// Control tools offered for this call. Default offers none.
    pub control: ControlToolConfig,
}

impl ToolLoopConfig {
    pub fn new(agent_workspace: &str, agent_path: &str) -> Self {
        Self {
            agent_workspace: agent_workspace.to_string(),
            agent_path: agent_path.to_string(),
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            response_format: None,
            control: ControlToolConfig::default(),
        }
    }

    /// Offer these control tools.
    pub fn with_control(mut self, control: ControlToolConfig) -> Self {
        self.control = control;
        self
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
    /// The payload the AI ended WITH — the whole point of ending deliberately.
    /// `None` when the session ended some other way (turn limit, user keyword).
    pub end_result: Option<Value>,
    /// The AI's stated reason for ending or handing off, for the audit trail.
    pub end_reason: Option<String>,
    /// Optional handoff target
    pub handoff_to: Option<String>,
    /// Set when the AI asked for a human decision and the flow must PARK.
    /// The caller is responsible for creating the task and parking; this loop
    /// only reports that it happened and stops.
    pub pending_approval: Option<PendingApproval>,
}

impl ToolLoopResult {
    /// An empty result carrying only the accumulated bookkeeping.
    fn bare(
        content: String,
        tool_calls_executed: Vec<ExecutedToolCall>,
        model: Option<String>,
        finish_reason: Option<String>,
        total_input_tokens: u64,
        total_output_tokens: u64,
    ) -> Self {
        Self {
            content,
            tool_calls_executed,
            model,
            finish_reason,
            total_input_tokens,
            total_output_tokens,
            end_session: false,
            end_result: None,
            end_reason: None,
            handoff_to: None,
            pending_approval: None,
        }
    }
}

/// A human decision the AI asked for mid-turn.
///
/// `tool_call_id` is the load-bearing field. The approval is answered by
/// persisting the human's response as the RESULT of that tool call, so that
/// `conversation_persistence::load_conversation_history` rebuilds the
/// `assistant(tool_calls)` / `role: tool` pair on the next turn and the model
/// simply sees its question answered. No side-car message state is kept.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub tool_call_id: String,
}

/// A single tool call that was executed.
pub struct ExecutedToolCall {
    pub id: String,
    pub function_name: String,
    pub arguments: Value,
    pub result: Value,
    pub error: Option<String>,
    pub duration_ms: u64,
    /// True while the call is still AWAITING its result (an approval the flow
    /// parked on). Persistence must not write a result child for one of these:
    /// a tool result in the history is what tells the model the question was
    /// answered.
    pub pending: bool,
}

impl ExecutedToolCall {
    /// A control tool's record — no function ran, but the call belongs in the
    /// audit trail exactly like any other.
    fn control(id: &str, name: &str, arguments: Value, result: Value) -> Self {
        Self {
            id: id.to_string(),
            function_name: name.to_string(),
            arguments,
            result,
            error: None,
            duration_ms: 0,
            pending: false,
        }
    }
}

/// What processing one batch of tool calls decided about the loop.
enum BatchOutcome {
    /// Every call was executed and fed back; keep looping.
    Continue,
    /// A control tool ended the turn.
    Stop(Box<ControlStop>),
}

/// The terminal state a control tool put the turn into.
#[derive(Default)]
struct ControlStop {
    end_session: bool,
    end_result: Option<Value>,
    end_reason: Option<String>,
    handoff_to: Option<String>,
    pending_approval: Option<PendingApproval>,
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
    let extra_tools = config.control.definitions();

    for iteration in 0..=config.max_tool_iterations {
        let response = callbacks
            .call_ai_with_tools(
                &config.agent_workspace,
                &config.agent_path,
                messages.clone(),
                config.response_format.clone(),
                extra_tools.clone(),
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

        accumulate_usage(&response, &mut total_input_tokens, &mut total_output_tokens);
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

            return Ok(finalize(
                &response,
                all_tool_calls,
                last_model,
                total_input_tokens,
                total_output_tokens,
            ));
        }

        let assistant_content = content_of(&response);
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
            "tool_calls": tool_calls,
        }));

        match process_tool_calls(
            callbacks,
            &tool_calls,
            &tool_map,
            instance_id,
            iteration,
            &config.control,
            &mut messages,
            &mut all_tool_calls,
        )
        .await
        {
            BatchOutcome::Continue => {}
            BatchOutcome::Stop(stop) => {
                return Ok(stopped(
                    *stop,
                    assistant_content,
                    all_tool_calls,
                    last_model,
                    total_input_tokens,
                    total_output_tokens,
                ))
            }
        }
    }

    // Should not reach here, but just in case
    Ok(ToolLoopResult::bare(
        String::new(),
        all_tool_calls,
        last_model,
        None,
        total_input_tokens,
        total_output_tokens,
    ))
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
    let extra_tools = config.control.definitions();

    for iteration in 0..=config.max_tool_iterations {
        let mut rx = callbacks
            .call_ai_streaming_with_tools(
                &config.agent_workspace,
                &config.agent_path,
                messages.clone(),
                config.response_format.clone(),
                extra_tools.clone(),
            )
            .await?;

        // Accumulate the full response from streaming chunks
        let response = accumulate_stream(&mut rx, callbacks, instance_id, &mut tool_map).await;

        accumulate_usage(&response, &mut total_input_tokens, &mut total_output_tokens);
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
            return Ok(finalize(
                &response,
                all_tool_calls,
                last_model,
                total_input_tokens,
                total_output_tokens,
            ));
        }

        // Tool calls present — execute them (same logic as non-streaming)
        let assistant_content = content_of(&response);
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
            "tool_calls": tool_calls,
        }));

        match process_tool_calls(
            callbacks,
            &tool_calls,
            &tool_map,
            instance_id,
            iteration,
            &config.control,
            &mut messages,
            &mut all_tool_calls,
        )
        .await
        {
            BatchOutcome::Continue => {}
            BatchOutcome::Stop(stop) => {
                return Ok(stopped(
                    *stop,
                    assistant_content,
                    all_tool_calls,
                    last_model,
                    total_input_tokens,
                    total_output_tokens,
                ))
            }
        }
    }

    Ok(ToolLoopResult::bare(
        String::new(),
        all_tool_calls,
        last_model,
        None,
        total_input_tokens,
        total_output_tokens,
    ))
}

/// Execute (or intercept) one batch of tool calls.
///
/// Control calls are recognised BEFORE `execute_function`, appended to
/// `all_tool_calls` for the audit trail, and stop the loop. Ordinary calls run
/// and their results are pushed onto `messages` as `role: tool`.
#[allow(clippy::too_many_arguments)]
async fn process_tool_calls(
    callbacks: &dyn FlowCallbacks,
    tool_calls: &[Value],
    tool_map: &std::collections::HashMap<String, String>,
    instance_id: &str,
    iteration: u32,
    control: &ControlToolConfig,
    messages: &mut Vec<Value>,
    all_tool_calls: &mut Vec<ExecutedToolCall>,
) -> BatchOutcome {
    for tc in tool_calls {
        let (tc_id, func_name, arguments) = describe_tool_call(tc);

        if !control_tools::is_control_tool(&func_name) {
            let executed =
                execute_tool_call(callbacks, tc, tool_map, instance_id, iteration).await;
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": executed.id,
                "content": serde_json::to_string(&executed.result).unwrap_or_default(),
            }));
            all_tool_calls.push(executed);
            continue;
        }

        // A control tool the step did not offer is not a control tool. The
        // model cannot have been shown it, so a call naming it is a
        // hallucination — reported back as a tool error, never acted on.
        if !is_offered(&func_name, control) {
            warn!(
                tool = %func_name,
                "Model called a control tool this step does not offer - refusing"
            );
            let result = serde_json::json!({
                "error": format!("`{}` is not available in this conversation.", func_name),
            });
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tc_id,
                "content": serde_json::to_string(&result).unwrap_or_default(),
            }));
            all_tool_calls.push(ExecutedToolCall::control(
                &tc_id,
                &func_name,
                arguments,
                result,
            ));
            continue;
        }

        let Some(action) = control_tools::interpret(&func_name, &arguments) else {
            // Malformed: tell the model and let it try again rather than
            // silently ending a conversation or parking a flow.
            let result = control_tools::malformed(&func_name);
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tc_id,
                "content": serde_json::to_string(&result).unwrap_or_default(),
            }));
            all_tool_calls.push(ExecutedToolCall::control(
                &tc_id,
                &func_name,
                arguments,
                result,
            ));
            continue;
        };

        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_started(&tc_id, &func_name, arguments.clone()),
            )
            .await;

        let mut stop = ControlStop::default();
        let result = match action {
            ControlAction::End { result, reason } => {
                info!(tool_call_id = %tc_id, "Agent ended the session");
                stop.end_session = true;
                stop.end_result = Some(result.clone());
                stop.end_reason = reason;
                serde_json::json!({ "ended": true, "result": result })
            }
            ControlAction::Handoff { agent, reason } => {
                info!(tool_call_id = %tc_id, agent = %agent, "Agent handed off");
                stop.handoff_to = Some(agent.clone());
                stop.end_reason = reason;
                serde_json::json!({ "handed_off_to": agent })
            }
            ControlAction::Approval(request) => {
                info!(tool_call_id = %tc_id, title = %request.title, "Agent requested approval");
                stop.pending_approval = Some(PendingApproval {
                    request,
                    tool_call_id: tc_id.clone(),
                });
                // Deliberately NO result: the human's answer becomes this
                // call's result when the flow resumes.
                Value::Null
            }
        };

        let pending = stop.pending_approval.is_some();
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_completed(
                    &tc_id,
                    result.clone(),
                    None,
                    Some(0),
                ),
            )
            .await;

        all_tool_calls.push(ExecutedToolCall {
            id: tc_id,
            function_name: func_name,
            arguments,
            result,
            error: None,
            duration_ms: 0,
            pending,
        });

        return BatchOutcome::Stop(Box::new(stop));
    }

    BatchOutcome::Continue
}

/// Is this control tool one the step actually offered?
fn is_offered(name: &str, control: &ControlToolConfig) -> bool {
    match name {
        control_tools::END_SESSION => control.allow_end,
        control_tools::HANDOFF => !control.handoff_targets.is_empty(),
        control_tools::REQUEST_APPROVAL => control.allow_approval,
        _ => false,
    }
}

/// Build the result for a turn that ended because a control tool fired.
fn stopped(
    stop: ControlStop,
    content: String,
    tool_calls_executed: Vec<ExecutedToolCall>,
    model: Option<String>,
    total_input_tokens: u64,
    total_output_tokens: u64,
) -> ToolLoopResult {
    ToolLoopResult {
        content,
        tool_calls_executed,
        model,
        finish_reason: Some("control_tool".to_string()),
        total_input_tokens,
        total_output_tokens,
        end_session: stop.end_session,
        end_result: stop.end_result,
        end_reason: stop.end_reason,
        handoff_to: stop.handoff_to,
        pending_approval: stop.pending_approval,
    }
}

/// Build the result for a turn that ended normally (no tool calls left).
///
/// `end_session` / `handoff_to` are still read off the response here: a
/// provider that signals them natively keeps working, and that legacy path is
/// now the SECOND way to end rather than the only (unreachable) one.
fn finalize(
    response: &Value,
    tool_calls_executed: Vec<ExecutedToolCall>,
    model: Option<String>,
    total_input_tokens: u64,
    total_output_tokens: u64,
) -> ToolLoopResult {
    ToolLoopResult {
        content: content_of(response),
        tool_calls_executed,
        model,
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
        end_result: response.get("end_result").cloned(),
        end_reason: None,
        handoff_to: response
            .get("handoff_to")
            .and_then(|v| v.as_str())
            .map(String::from),
        pending_approval: None,
    }
}

/// The assistant text of a response, under either key providers use.
fn content_of(response: &Value) -> String {
    response
        .get("content")
        .or_else(|| response.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Add a response's token usage to the running totals.
fn accumulate_usage(response: &Value, input: &mut u64, output: &mut u64) {
    if let Some(usage) = response.get("usage") {
        *input += usage
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        *output += usage
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
    }
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

/// Pull `(id, name, arguments)` out of a raw tool call.
fn describe_tool_call(tc: &Value) -> (String, String, Value) {
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
    (tc_id, func_name, parse_tool_arguments(&func))
}

/// Execute a single tool call, emitting start/complete events.
async fn execute_tool_call(
    callbacks: &dyn FlowCallbacks,
    tc: &Value,
    tool_map: &std::collections::HashMap<String, String>,
    instance_id: &str,
    iteration: u32,
) -> ExecutedToolCall {
    let (tc_id, func_name, arguments) = describe_tool_call(tc);

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
        pending: false,
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
