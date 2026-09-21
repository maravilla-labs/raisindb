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
use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowNode, FlowResult};
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Maximum tool-loop iterations to prevent runaway.
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 10;

/// The builtin tool that loads a `raisin:Skill` body.
pub const LOAD_SKILL_TOOL: &str = "load-skill";

/// A step's own `skills:` — references added to its agent's skills for that
/// step only. Anything but an array is none.
pub fn step_skills(step: &FlowNode) -> Vec<Value> {
    step.get_array("skills").cloned().unwrap_or_default()
}

/// The builtin `load-skill` function's path in the `functions` workspace.
pub const LOAD_SKILL_PATH: &str = "/lib/raisin/ai/load-skill";

/// The key a function STEP's arguments carry the flow that ran it under:
/// `{ "instance_id", "step_id" }`. Only the runtime sets it (see
/// `function_step`), so a function can tie what it does to ITS flow instance —
/// an approval to the arming flow that asked for it, say.
pub const FLOW_CONTEXT_KEY: &str = "__raisin_flow";

/// Argument keys only the runtime may set. A model that writes one is forging.
const RUNTIME_KEYS: [&str; 3] = ["__raisin_context", "_skill_grant", FLOW_CONTEXT_KEY];

/// The function path an OFFERED tool resolves to — `None` for a name the
/// model was never offered.
///
/// `tool_map` is the callback's `_tool_map` for this call: every function tool
/// the agent was given, name → path (`load-skill` included when skills were
/// granted). It is the whole offer. A name outside it used to fall back to
/// being executed as a function PATH, so a model that wrote
/// `/lib/studio/anything` ran that function with the flow's authority whether
/// or not it had been given it. Every path that executes a model's tool call
/// asks this first and refuses on `None`.
pub fn offered_function<'a>(
    name: &str,
    tool_map: &'a std::collections::HashMap<String, String>,
) -> Option<&'a str> {
    tool_map.get(name).map(String::as_str)
}

/// The error a model reads back when it calls a tool it was not offered: what
/// happened (nothing ran) and what it CAN call, so the next turn can recover.
pub fn unoffered_tool_error(
    name: &str,
    tool_map: &std::collections::HashMap<String, String>,
) -> String {
    let mut offered: Vec<&str> = tool_map.keys().map(String::as_str).collect();
    offered.sort_unstable();
    let choices = if offered.is_empty() {
        "No tools are offered in this step.".to_string()
    } else {
        format!("The tools offered to you are: {}.", offered.join(", "))
    };
    format!(
        "`{}` is not a tool offered to you, so it was not run. {}",
        name, choices
    )
}

/// The callback's `_tool_map` on one response. Absent or malformed is EMPTY:
/// the callback leaves the key out exactly when it offered no function tools.
pub fn tool_map_of(response: &Value) -> std::collections::HashMap<String, String> {
    response
        .get("_tool_map")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default()
}

/// Remove every runtime-only key from arguments the MODEL wrote. Anything but
/// an object passes through unchanged.
pub fn strip_runtime_keys(mut arguments: Value) -> Value {
    if let Some(obj) = arguments.as_object_mut() {
        for key in RUNTIME_KEYS {
            obj.remove(key);
        }
    }
    arguments
}

/// Whether a call reaches `load-skill`: by the tool name the model used, or by
/// the function the name resolved to (bare or workspace-qualified path). A
/// model can name an unmapped function by its path, and the runtime then runs
/// that path, so the NAME alone is not enough.
pub fn is_load_skill(name: &str, function_ref: &str) -> bool {
    name == LOAD_SKILL_TOOL
        || function_ref
            .trim_end_matches('/')
            .ends_with(LOAD_SKILL_PATH)
}

/// The arguments a tool call is executed with.
///
/// The model's arguments lose every runtime-only key ([`RUNTIME_KEYS`]) — for
/// EVERY tool, since nothing legitimate in a model's tool call carries one.
/// A `load-skill` call (by tool name or by the function path it resolved to)
/// then gets `__raisin_context = {skill_grant}` — the grant the AI callback
/// resolved (`_skill_grant` on its response). A flow has no chat for the tool
/// to derive the grant from, so this is the only grant it sees; a missing one
/// is empty and the tool refuses.
pub fn tool_arguments(
    name: &str,
    function_ref: &str,
    arguments: Value,
    skill_grant: Option<&Value>,
) -> Value {
    let mut arguments = strip_runtime_keys(arguments);
    if !is_load_skill(name, function_ref) {
        return arguments;
    }
    if !arguments.is_object() {
        arguments = Value::Object(Default::default());
    }
    let grant = skill_grant
        .filter(|g| g.is_array())
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    arguments["__raisin_context"] = serde_json::json!({ "skill_grant": grant });
    arguments
}

/// Configuration for an AI-with-tools invocation.
pub struct ToolLoopConfig {
    pub agent_workspace: String,
    pub agent_path: String,
    pub max_tool_iterations: u32,
    pub response_format: Option<Value>,
    /// Control tools offered for this call. Default offers none.
    pub control: ControlToolConfig,
    /// The step's own skill references, added to the agent's for this call.
    pub skills: Vec<Value>,
}

impl ToolLoopConfig {
    pub fn new(agent_workspace: &str, agent_path: &str) -> Self {
        Self {
            agent_workspace: agent_workspace.to_string(),
            agent_path: agent_path.to_string(),
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            response_format: None,
            control: ControlToolConfig::default(),
            skills: Vec::new(),
        }
    }

    /// Offer these control tools.
    pub fn with_control(mut self, control: ControlToolConfig) -> Self {
        self.control = control;
        self
    }

    /// Add the step's own skills to the agent's.
    pub fn with_skills(mut self, skills: Vec<Value>) -> Self {
        self.skills = skills;
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
    let extra_tools = config.control.definitions();

    for iteration in 0..=config.max_tool_iterations {
        let response = callbacks
            .call_ai_with_options(
                &config.agent_workspace,
                &config.agent_path,
                messages.clone(),
                config.response_format.clone(),
                extra_tools.clone(),
                config.skills.clone(),
            )
            .await?;
        let skill_grant = response.get("_skill_grant").cloned();

        // Tool name → path: THIS call's offer (included by ai_callback). Read
        // per response, so a call is judged against what it was offered.
        let tool_map = tool_map_of(&response);

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
            skill_grant.as_ref(),
            instance_id,
            iteration,
            &config.agent_path,
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
    let extra_tools = config.control.definitions();

    for iteration in 0..=config.max_tool_iterations {
        let rx = callbacks
            .call_ai_streaming_with_options(
                &config.agent_workspace,
                &config.agent_path,
                messages.clone(),
                config.response_format.clone(),
                extra_tools.clone(),
                config.skills.clone(),
            )
            .await?;

        // Accumulate the full response from streaming chunks, keeping the
        // side-band `_skill_grant` the accumulator does not pass through. The
        // accumulator's own map is CALL ID -> name, which is why it is kept
        // apart: merged into the offer, a call id the model chose became a
        // "tool" it could name.
        let mut call_names = std::collections::HashMap::new();
        let (response, skill_grant) =
            accumulate_stream_with_grant(rx, callbacks, instance_id, &mut call_names).await;
        // Tool NAME -> function PATH: this call's offer, off the response.
        let tool_map = tool_map_of(&response);

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
            skill_grant.as_ref(),
            instance_id,
            iteration,
            &config.agent_path,
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
    skill_grant: Option<&Value>,
    instance_id: &str,
    iteration: u32,
    agent_path: &str,
    control: &ControlToolConfig,
    messages: &mut Vec<Value>,
    all_tool_calls: &mut Vec<ExecutedToolCall>,
) -> BatchOutcome {
    for tc in tool_calls {
        let (tc_id, func_name, arguments) = describe_tool_call(tc);

        if !control_tools::is_control_tool(&func_name) {
            let executed = execute_tool_call(
                callbacks,
                tc,
                tool_map,
                skill_grant,
                instance_id,
                iteration,
                agent_path,
            )
            .await;
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
                &tc_id, &func_name, arguments, result,
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
                &tc_id, &func_name, arguments, result,
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
                FlowExecutionEvent::tool_call_completed(&tc_id, result.clone(), None, Some(0)),
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

/// [`accumulate_stream`], also returning the `_skill_grant` the callback sent
/// on its first chunk (`raisin_ai`'s accumulator keeps only `_tool_map`).
///
/// The receiver is tapped rather than read ahead, so every chunk still
/// reaches the accumulator in order.
async fn accumulate_stream_with_grant(
    mut rx: tokio::sync::mpsc::Receiver<Value>,
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    tool_map: &mut std::collections::HashMap<String, String>,
) -> (Value, Option<Value>) {
    let (tx, mut tapped) = tokio::sync::mpsc::channel::<Value>(32);
    let forward = async move {
        let mut grant: Option<Value> = None;
        while let Some(chunk) = rx.recv().await {
            if grant.is_none() {
                grant = chunk.get("_skill_grant").cloned();
            }
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
        grant
    };
    let (grant, response) = tokio::join!(
        forward,
        accumulate_stream(&mut tapped, callbacks, instance_id, tool_map)
    );
    (response, grant)
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
    skill_grant: Option<&Value>,
    instance_id: &str,
    iteration: u32,
    agent_path: &str,
) -> ExecutedToolCall {
    let (tc_id, func_name, arguments) = describe_tool_call(tc);

    debug!(tool_call_id = %tc_id, function = %func_name, "Executing tool call (iteration {})", iteration);

    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::tool_call_started(&tc_id, &func_name, arguments.clone()),
        )
        .await;

    // Only a tool the agent was OFFERED runs. Anything else is refused back
    // to the model as a tool error it can read and recover from.
    let Some(func_path) = offered_function(&func_name, tool_map).map(String::from) else {
        let message = unoffered_tool_error(&func_name, tool_map);
        warn!(tool_call_id = %tc_id, tool = %func_name, "Model called a tool it was not offered - refusing");
        let result_value = serde_json::json!({ "error": message });
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::tool_call_completed(
                    &tc_id,
                    result_value.clone(),
                    Some(message.clone()),
                    Some(0),
                ),
            )
            .await;
        return ExecutedToolCall {
            id: tc_id,
            function_name: func_name,
            arguments,
            result: result_value,
            error: Some(message),
            duration_ms: 0,
            pending: false,
        };
    };

    let start = Instant::now();
    // AS THE AGENT: its own tool call, so its own configured rights.
    let exec_result = callbacks
        .execute_function_as_agent(
            &func_path,
            tool_arguments(&func_name, &func_path, arguments.clone(), skill_grant),
            agent_path,
        )
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

#[cfg(test)]
mod skill_grant_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_skill_gets_the_runtime_grant_not_the_models() {
        let grant = json!([{ "name": "pdf", "workspace": "functions", "path": "/skills/pdf" }]);
        let args = tool_arguments(
            LOAD_SKILL_TOOL,
            LOAD_SKILL_PATH,
            json!({ "name": "pdf", "__raisin_context": { "skill_grant": [{ "name": "forged" }], "chat_path": "/x" } }),
            Some(&grant),
        );
        assert_eq!(args["name"], "pdf");
        assert_eq!(args["__raisin_context"], json!({ "skill_grant": grant }));
    }

    #[test]
    fn a_missing_grant_is_empty() {
        let args = tool_arguments(LOAD_SKILL_TOOL, LOAD_SKILL_PATH, Value::Null, None);
        assert_eq!(args, json!({ "__raisin_context": { "skill_grant": [] } }));
    }

    #[test]
    fn other_tools_lose_runtime_keys_and_keep_the_rest() {
        let original =
            json!({ "__raisin_context": { "chat_path": "/x" }, "_skill_grant": [], "q": "x" });
        assert_eq!(
            tool_arguments(
                "lookup",
                "/lib/x/lookup",
                original,
                Some(&json!([{ "name": "a" }]))
            ),
            json!({ "q": "x" })
        );
        // A non-object passes through untouched.
        assert_eq!(
            tool_arguments("lookup", "/lib/x/lookup", json!("s"), None),
            json!("s")
        );
    }

    #[test]
    fn load_skill_named_by_its_path_still_gets_only_the_runtime_grant() {
        let grant =
            json!([{ "name": "granted", "workspace": "functions", "path": "/skills/granted" }]);
        let forged = json!({
            "name": "secret",
            "__raisin_context": { "skill_grant": [{ "name": "secret", "path": "/skills/secret" }], "chat_path": "/c" },
            "_skill_grant": [{ "name": "secret" }],
        });
        for (name, path) in [
            (LOAD_SKILL_PATH, LOAD_SKILL_PATH),
            (
                "functions:/lib/raisin/ai/load-skill",
                "functions:/lib/raisin/ai/load-skill",
            ),
            ("renamed", "/lib/raisin/ai/load-skill/"),
        ] {
            let args = tool_arguments(name, path, forged.clone(), Some(&grant));
            assert_eq!(
                args,
                json!({ "name": "secret", "__raisin_context": { "skill_grant": grant } }),
                "{name} -> {path}"
            );
        }
    }
}

/// A scripted callback shared by the tests of every path that executes a
/// model's tool calls (this loop, `agent_step`, the AI container).
#[cfg(test)]
pub(crate) mod test_support {
    use crate::types::{FlowCallbacks, FlowInstance, FlowResult};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    /// One function execution: the path, the arguments, and the AGENT it ran
    /// as (`None` = plain `execute_function`, i.e. with the flow's authority).
    pub(crate) type Executed = (String, Value, Option<String>);

    /// Returns the scripted AI responses in order (then a plain `stop`), and
    /// records every function executed.
    pub(crate) struct ScriptedCallbacks {
        responses: Mutex<Vec<Value>>,
        executed: Arc<Mutex<Vec<Executed>>>,
    }

    impl ScriptedCallbacks {
        pub(crate) fn new(responses: Vec<Value>) -> Self {
            Self {
                responses: Mutex::new(responses),
                executed: Arc::default(),
            }
        }
        pub(crate) fn executed(&self) -> Vec<Executed> {
            self.executed.lock().unwrap().clone()
        }
    }

    /// An AI turn calling `calls` (`(id, name, arguments)`), with `offer` as
    /// the callback's `_tool_map` — `None` leaves the key out, which is what
    /// the real callback does when it offers no function tools.
    pub(crate) fn turn(calls: &[(&str, &str, Value)], offer: Option<Value>) -> Value {
        let tool_calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(index, (id, name, args))| {
                // `index` keeps calls apart on the streaming path, which
                // merges deltas by it.
                json!({
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": args.to_string() }
                })
            })
            .collect();
        let mut response = json!({
            "content": "",
            "finish_reason": "tool_calls",
            "tool_calls": tool_calls,
        });
        if let Some(offer) = offer {
            response["_tool_map"] = offer;
        }
        response
    }

    #[async_trait]
    impl FlowCallbacks for ScriptedCallbacks {
        async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
            unimplemented!()
        }
        async fn save_instance(&self, _instance: &FlowInstance) -> FlowResult<()> {
            Ok(())
        }
        async fn save_instance_with_version(
            &self,
            _instance: &FlowInstance,
            _expected_version: i32,
        ) -> FlowResult<()> {
            Ok(())
        }
        async fn create_node(&self, _t: &str, _p: &str, _props: Value) -> FlowResult<Value> {
            Ok(json!({}))
        }
        async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
            Ok(json!({}))
        }
        async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
            Ok(None)
        }
        async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
            Ok("job-1".to_string())
        }
        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<Value>,
            _response_format: Option<Value>,
        ) -> FlowResult<Value> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Ok(json!({ "content": "done", "finish_reason": "stop" }));
            }
            Ok(responses.remove(0))
        }
        async fn execute_function(&self, function_ref: &str, input: Value) -> FlowResult<Value> {
            self.executed
                .lock()
                .unwrap()
                .push((function_ref.to_string(), input, None));
            Ok(json!({ "ran": function_ref }))
        }
        async fn execute_function_as_agent(
            &self,
            function_ref: &str,
            input: Value,
            agent_path: &str,
        ) -> FlowResult<Value> {
            self.executed.lock().unwrap().push((
                function_ref.to_string(),
                input,
                Some(agent_path.to_string()),
            ));
            Ok(json!({ "ran": function_ref }))
        }
    }
}

#[cfg(test)]
mod offered_tool_tests {
    use super::test_support::{turn, ScriptedCallbacks};
    use super::*;
    use serde_json::json;

    fn config() -> ToolLoopConfig {
        ToolLoopConfig::new("functions", "/agents/writer")
    }

    fn user() -> Vec<Value> {
        vec![json!({ "role": "user", "content": "go" })]
    }

    /// The model names a function it was never offered — by its PATH, the old
    /// fallback that ran anything. Nothing runs; the model reads a refusal.
    #[tokio::test]
    async fn an_unoffered_function_is_refused_not_run() {
        let offer = json!({ "lookup": "/lib/x/lookup" });
        let callbacks = ScriptedCallbacks::new(vec![turn(
            &[(
                "c1",
                "/lib/studio/automations/materialize-automation",
                json!({}),
            )],
            Some(offer),
        )]);
        let out = run_ai_with_tools(&callbacks, user(), &config(), "i-1")
            .await
            .expect("loop runs");

        assert!(
            callbacks.executed().is_empty(),
            "{:?}",
            callbacks.executed()
        );
        let refused = &out.tool_calls_executed[0];
        let error = refused.error.as_deref().expect("refused with an error");
        assert!(error.contains("is not a tool offered to you"), "{error}");
        assert!(error.contains("lookup"), "names what it can call: {error}");
        assert_eq!(out.content, "done", "the loop went on to the next turn");
    }

    #[tokio::test]
    async fn no_offer_at_all_refuses_every_call() {
        let callbacks =
            ScriptedCallbacks::new(vec![turn(&[("c1", "update-node", json!({}))], None)]);
        let out = run_ai_with_tools(&callbacks, user(), &config(), "i-1")
            .await
            .expect("loop runs");
        assert!(callbacks.executed().is_empty());
        assert!(out.tool_calls_executed[0]
            .error
            .as_deref()
            .unwrap()
            .contains("No tools are offered"));
    }

    #[tokio::test]
    async fn an_offered_tool_runs_as_the_agent() {
        let callbacks = ScriptedCallbacks::new(vec![turn(
            &[(
                "c1",
                "lookup",
                json!({ "q": "x", "__raisin_flow": { "instance_id": "forged" } }),
            )],
            Some(json!({ "lookup": "/lib/x/lookup" })),
        )]);
        run_ai_with_tools(&callbacks, user(), &config(), "i-1")
            .await
            .expect("loop runs");
        assert_eq!(
            callbacks.executed(),
            vec![(
                "/lib/x/lookup".to_string(),
                // The model cannot forge a flow stamp either.
                json!({ "q": "x" }),
                Some("/agents/writer".to_string())
            )]
        );
    }

    /// Streaming: the accumulator's own map is CALL ID -> name. Merged into the
    /// offer (as it used to be), a call id became a callable "tool" name.
    #[tokio::test]
    async fn streaming_a_call_id_is_not_a_tool_name() {
        let offer = json!({ "lookup": "/lib/x/lookup" });
        let callbacks = ScriptedCallbacks::new(vec![
            turn(&[("lookup-2", "lookup", json!({}))], Some(offer.clone())),
            // The next turn names the previous call's ID as a tool.
            turn(&[("c9", "lookup-2", json!({}))], Some(offer)),
        ]);
        let out = run_ai_with_tools_streaming(&callbacks, user(), &config(), "i-1")
            .await
            .expect("loop runs");
        assert_eq!(callbacks.executed().len(), 1, "only the offered call ran");
        assert!(out.tool_calls_executed[1].error.is_some());
    }
}
