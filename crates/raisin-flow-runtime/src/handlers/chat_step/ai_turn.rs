// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI turn processing: build messages from the node tree, call the tool loop,
//! and persist the result back.

use super::types::{ChatConfig, ChatSessionState};
use crate::handlers::ai_tool_loop::{self, ToolLoopConfig, ToolLoopResult};
use crate::handlers::control_tools::ControlToolConfig;
use crate::handlers::conversation_persistence::{
    self, AiResponseData, ToolCallData, UsageData, AI_SENDER_ID,
};
use crate::types::{AiExecutionConfig, FlowCallbacks, FlowContext, FlowExecutionEvent, FlowResult};
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Run one AI turn: build messages, call the tool loop, persist result.
pub(super) async fn process_ai_turn(
    callbacks: &dyn FlowCallbacks,
    context: &FlowContext,
    config: &ChatConfig,
    session: &mut ChatSessionState,
    agent_path: &str,
    conv_workspace: &str,
    step_id: &str,
) -> FlowResult<()> {
    let ai_messages = build_ai_messages(
        callbacks,
        config,
        conv_workspace,
        session.conversation_path.as_deref().unwrap_or(""),
    )
    .await;

    let workspace = config.agent_workspace.as_deref().unwrap_or("functions");
    let tool_config =
        ToolLoopConfig::new(workspace, agent_path).with_control(control_config(config));

    let _ = callbacks
        .emit_event(
            &context.instance_id,
            FlowExecutionEvent::log(
                "info",
                format!("AI call starting for agent '{}'", agent_path),
                Some(step_id.to_string()),
            ),
        )
        .await;

    let result = call_ai_with_resilience(
        callbacks,
        ai_messages,
        &tool_config,
        &context.instance_id,
        &config.execution,
    )
    .await;

    match result {
        Ok(result) => {
            // Note: text_chunk events are emitted in real-time by the streaming tool loop,
            // so we do NOT emit a post-hoc text_chunk here.

            if let Some(ref new_agent) = result.handoff_to {
                info!("Chat session handing off to: {}", new_agent);
                session.current_agent = Some(new_agent.clone());
            }

            if result.end_session && config.termination.allow_ai_end {
                session.is_complete = true;
                session.completion_reason = Some("ai_terminated".to_string());
                // The RESULT, not just the fact of ending. `end_result` is
                // `None` when a provider signalled `end_session` natively
                // without one; `Null` when the agent called the tool and passed
                // nothing. Both are recorded as they are — collapsing them
                // would lose the difference between "no payload was possible"
                // and "the agent had nothing to say".
                session.result = result.end_result.clone();
            } else if result.end_session {
                // The step forbids AI termination. Say so in the log rather
                // than silently continuing: the agent believes it finished.
                warn!(
                    "Agent asked to end the session but `termination.allow_ai_end` is off - continuing"
                );
            }

            let total_tokens = result.total_input_tokens + result.total_output_tokens;
            if total_tokens > 0 {
                session.total_tokens += total_tokens;
            }

            let message_path = match session.conversation_path.clone() {
                Some(conv_path) => {
                    persist_ai_result(
                        callbacks,
                        &context.instance_id,
                        &conv_path,
                        conv_workspace,
                        &result,
                    )
                    .await
                }
                None => None,
            };

            // An approval can only be parked once the assistant message exists:
            // the pending tool call's result is written UNDER it, and the path
            // is what the resume needs.
            if let Some(pending) = &result.pending_approval {
                match message_path {
                    Some(ref path) => {
                        session.pending_approval = super::approval::open(
                            callbacks,
                            context,
                            session,
                            &config.approval,
                            step_id,
                            path,
                            pending,
                        )
                        .await;
                    }
                    None => warn!(
                        "Agent requested approval but the assistant message was not persisted - \
                         not parking"
                    ),
                }
            }

            Ok(())
        }
        Err(e) => {
            let error_msg = format!("AI agent call failed for '{}': {}", agent_path, e);
            warn!("{}", error_msg);

            let _ = callbacks
                .emit_event(
                    &context.instance_id,
                    FlowExecutionEvent::log("error", &error_msg, Some(step_id.to_string())),
                )
                .await;

            // Persist error as an assistant message so the user sees it in the conversation
            if let Some(ref conv_path) = session.conversation_path {
                let error_content = format!("Error: Agent temporarily unavailable ({})", e);
                let response_data = AiResponseData {
                    content: error_content.clone(),
                    model: None,
                    finish_reason: Some("error".to_string()),
                    tool_calls: Vec::new(),
                    usage: None,
                    thinking: Vec::new(),
                };
                let _ = conversation_persistence::persist_assistant_response(
                    callbacks,
                    &context.instance_id,
                    conv_path,
                    conv_workspace,
                    &response_data,
                )
                .await;
                // Update last_message so the conversation list UI shows the error
                let _ = conversation_persistence::update_conversation_last_message(
                    callbacks,
                    conv_workspace,
                    conv_path,
                    &error_content,
                    AI_SENDER_ID,
                )
                .await;
            }

            Err(e)
        }
    }
}

/// Call the AI tool loop with retry and timeout resilience.
///
/// Wraps the tool loop in a per-call timeout (if configured) and retries
/// transient failures with exponential backoff.
async fn call_ai_with_resilience(
    callbacks: &dyn FlowCallbacks,
    messages: Vec<serde_json::Value>,
    config: &ToolLoopConfig,
    instance_id: &str,
    exec: &AiExecutionConfig,
) -> FlowResult<ToolLoopResult> {
    let max_attempts = exec.max_retries + 1;

    for attempt in 0..max_attempts {
        let fut = ai_tool_loop::run_ai_with_tools_streaming(
            callbacks,
            messages.clone(),
            config,
            instance_id,
        );

        let result = if let Some(timeout_ms) = exec.timeout_ms {
            match tokio::time::timeout(Duration::from_millis(timeout_ms), fut).await {
                Ok(inner) => inner,
                Err(_) => {
                    warn!(
                        "AI call timed out after {}ms (attempt {}/{})",
                        timeout_ms,
                        attempt + 1,
                        max_attempts
                    );
                    Err(crate::types::FlowError::TimeoutExceeded {
                        duration_ms: timeout_ms,
                    })
                }
            }
        } else {
            fut.await
        };

        match result {
            Ok(r) => return Ok(r),
            Err(e) if attempt + 1 < max_attempts => {
                let delay = exec.retry_delay_ms * 2u64.pow(attempt);
                warn!(
                    "AI call failed (attempt {}/{}), retrying in {}ms: {}",
                    attempt + 1,
                    max_attempts,
                    delay,
                    e
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!("loop always returns")
}

/// Build AI messages by loading full conversation history from the node tree.
async fn build_ai_messages(
    callbacks: &dyn FlowCallbacks,
    config: &ChatConfig,
    conv_workspace: &str,
    conversation_path: &str,
) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();

    if let Some(ref system_prompt) = config.system_prompt {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }));
    }

    if !config.handoff_targets.is_empty() {
        let handoff_context = config
            .handoff_targets
            .iter()
            .map(|t| {
                format!(
                    "- {}: {}",
                    t.agent_ref,
                    t.description.as_deref().unwrap_or("Available for handoff")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        messages.push(serde_json::json!({
            "role": "system",
            "content": format!(
                "When another agent is better suited, call the `handoff_to_agent` tool with \
                 one of these:\n{}",
                handoff_context
            ),
        }));
    }

    if !conversation_path.is_empty() {
        debug!(
            "Loading conversation history: workspace='{}', path='{}', type={:?}",
            conv_workspace, conversation_path, config.conversation_type
        );
        match conversation_persistence::load_conversation_history(
            callbacks,
            conv_workspace,
            conversation_path,
        )
        .await
        {
            Ok(history) => {
                debug!(
                    "Loaded {} history messages from '{}'",
                    history.len(),
                    conversation_path
                );
                messages.extend(history);
            }
            Err(e) => warn!("Failed to load conversation history: {}", e),
        }
    } else {
        debug!("No conversation path — skipping history load");
    }

    messages
}

/// Persist an AI tool-loop result to the conversation node tree.
///
/// Uses the unified `raisin:Message` format with tool call children and
/// cost records, then updates the conversation's `last_message` preview.
/// Persist an AI tool-loop result and return the assistant message's path.
async fn persist_ai_result(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    conv_path: &str,
    conv_workspace: &str,
    result: &ai_tool_loop::ToolLoopResult,
) -> Option<String> {
    let total_tokens = result.total_input_tokens + result.total_output_tokens;
    let usage = (total_tokens > 0).then(|| UsageData {
        input_tokens: Some(result.total_input_tokens),
        output_tokens: Some(result.total_output_tokens),
        model: result.model.clone(),
        provider: None,
    });

    let tool_calls: Vec<ToolCallData> = result
        .tool_calls_executed
        .iter()
        .map(|tc| ToolCallData {
            id: tc.id.clone(),
            function_name: tc.function_name.clone(),
            arguments: tc.arguments.clone(),
            error: tc.error.clone(),
        })
        .collect();

    let response_data = AiResponseData {
        content: result.content.clone(),
        model: result.model.clone(),
        finish_reason: result.finish_reason.clone(),
        tool_calls,
        usage,
        thinking: Vec::new(),
    };

    let message_path = match conversation_persistence::persist_assistant_response(
        callbacks,
        instance_id,
        conv_path,
        conv_workspace,
        &response_data,
    )
    .await
    {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to persist assistant response: {}", e);
            return None;
        }
    };

    for tc in &result.tool_calls_executed {
        // A PENDING call has no result yet — the human has not answered. Writing
        // one now would tell the next turn's history rebuild the question was
        // already resolved, and the agent would carry on without the answer.
        if tc.pending {
            continue;
        }
        let tc_path = format!("{}/{}", message_path, tc.id);
        let _ = conversation_persistence::persist_tool_result(
            callbacks,
            conv_workspace,
            &tc_path,
            &tc.result,
            tc.error.as_deref(),
            Some(tc.duration_ms),
        )
        .await;
    }

    // Update last_message preview on the conversation node
    let _ = conversation_persistence::update_conversation_last_message(
        callbacks,
        conv_workspace,
        conv_path,
        &result.content,
        AI_SENDER_ID,
    )
    .await;

    Some(message_path)
}

/// Which control tools this chat step offers its agent.
///
/// Each is gated on the author having asked for it, and `handoff_targets`
/// doubles as the tool's `enum`, so an agent can only name an agent the step
/// listed.
fn control_config(config: &ChatConfig) -> ControlToolConfig {
    ControlToolConfig {
        allow_end: config.termination.allow_ai_end,
        result_schema: config.output_schema.clone(),
        handoff_targets: config
            .handoff_targets
            .iter()
            .map(|t| {
                (
                    t.agent_ref.clone(),
                    t.description
                        .clone()
                        .unwrap_or_else(|| "Available for handoff".to_string()),
                )
            })
            .collect(),
        allow_approval: config.approval.enabled,
        approval_assignee: config.approval.assignee.clone(),
    }
}
