// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Session lifecycle helpers: load/save state, resolve conversation location,
//! extract user messages, and build completion outputs.

use super::types::{ChatConfig, ChatSessionState};
use crate::handlers::conversation_persistence;
use crate::types::{FlowCallbacks, FlowContext, FlowNode};

use super::StepResult;

/// Merge step config with flow-input overrides for agent_ref / agent_workspace.
pub(super) fn resolve_config(mut config: ChatConfig, context: &FlowContext) -> ChatConfig {
    if config.agent_ref.is_none() {
        if let Some(agent) = context.input.get("agent").and_then(|v| v.as_str()) {
            config.agent_ref = Some(agent.to_string());
        }
    }
    if config.agent_workspace.is_none() {
        if let Some(ws) = context
            .input
            .get("agent_workspace")
            .and_then(|v| v.as_str())
        {
            config.agent_workspace = Some(ws.to_string());
        }
    }
    config
}

/// Load existing session state from flow variables, or create a new one.
pub(super) fn load_or_init_session(
    context: &FlowContext,
    session_key: &str,
    config: &ChatConfig,
) -> (bool, ChatSessionState) {
    match context
        .variables
        .get(session_key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
    {
        Some(state) => (false, state),
        None => (
            true,
            ChatSessionState {
                session_id: uuid::Uuid::new_v4().to_string(),
                current_agent: config.agent_ref.clone(),
                ..Default::default()
            },
        ),
    }
}

/// Extract and consume `__chat_user_message` from context variables.
///
/// On resume turns, the message is in `__chat_user_message` (set by resume_flow).
/// On the first turn, the message lives in `context.input["message"]` (set by run_flow).
/// A `__chat_input_consumed` guard prevents re-reading `context.input` on re-execution.
pub(super) fn take_user_message(context: &mut FlowContext) -> Option<String> {
    // Check __chat_user_message (set by resume_flow for subsequent turns)
    let msg = context
        .variables
        .get("__chat_user_message")
        .and_then(|v| v.as_str())
        .map(String::from);
    if msg.is_some() {
        context.variables.remove("__chat_user_message");
        return msg;
    }

    // Fallback: check context.input for first-turn message (set by run_flow).
    // Only consume once (guard against re-execution reading it again).
    if context.variables.contains_key("__chat_input_consumed") {
        return None;
    }
    let msg = context
        .input
        .get("message")
        .or_else(|| context.input.get("content"))
        .and_then(|v| v.as_str())
        .map(String::from);
    if msg.is_some() {
        context.variables.insert(
            "__chat_input_consumed".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    msg
}

/// Build a `StepResult::Continue` with session summary output.
pub(super) fn make_continue(step: &FlowNode, session: &ChatSessionState) -> StepResult {
    StepResult::Continue {
        next_node_id: step
            .next_node
            .clone()
            .unwrap_or_else(|| "__implicit_end__".to_string()),
        output: serde_json::json!({
            "session_id": session.session_id,
            "turn_count": session.turn_count,
            "completion_reason": session.completion_reason,
        }),
    }
}

/// Update conversation stats, then save session state to flow variables.
pub(super) async fn update_stats_and_save(
    callbacks: &dyn FlowCallbacks,
    context: &mut FlowContext,
    session_key: &str,
    session: &ChatSessionState,
    conv_workspace: &str,
) {
    if let Some(ref conv_path) = session.conversation_path {
        let _ = conversation_persistence::update_conversation_stats(
            callbacks,
            conv_workspace,
            conv_path,
            session.turn_count,
            session.total_tokens,
            "completed",
        )
        .await;
    }
    context.variables.insert(
        session_key.to_string(),
        serde_json::to_value(session).unwrap_or_default(),
    );
}

/// Determine the workspace and path prefix for conversation nodes.
///
/// If the flow was triggered by a real user (identified via `trigger_info`),
/// the conversation is stored in `raisin:access_control` under the user's
/// home path. Otherwise it falls back to `raisin:system`.
pub(super) fn resolve_conversation_location(context: &FlowContext) -> (String, String) {
    use conversation_persistence::{SYSTEM_WORKSPACE, USER_WORKSPACE};

    if let Some(ref trigger) = context.trigger_info {
        if trigger.event_type == crate::types::TriggerEventType::Manual {
            let actor = &trigger.node_id;
            if !actor.is_empty() && actor != "http_api" && actor != "system" && actor != "test_api"
            {
                if let Some(ref home) = trigger.node_path {
                    if !home.is_empty() && home.starts_with("/users/") && !home.contains("..") {
                        return (USER_WORKSPACE.to_string(), home.clone());
                    }
                }
                return (USER_WORKSPACE.to_string(), format!("/users/{}", actor));
            }
        }
    }

    (SYSTEM_WORKSPACE.to_string(), String::new())
}
