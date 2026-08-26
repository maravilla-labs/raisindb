// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Agent-initiated approval: the chat step's half of the `request_approval`
//! control tool.
//!
//! # Why the answer is a TOOL RESULT
//!
//! The obvious design is to park the flow, then hand the human's answer to the
//! agent as a new user message. That loses the question: the model sees an
//! answer with no record of having asked, and its own pending tool call sits
//! unresolved in the history — which most providers reject outright.
//!
//! So the answer is written as the RESULT of the very tool call that asked for
//! it. `conversation_persistence::load_conversation_history` already
//! reconstructs `assistant(tool_calls)` followed by `role: tool` for each
//! result, so the next turn rebuilds the exchange with no side-car state at
//! all: the model asked, and now it sees the reply. That the history loader
//! already did this is what makes agent-initiated approval a small change
//! rather than a new persistence format.

use super::types::{ApprovalConfig, ChatSessionState, PendingApprovalState};
use crate::handlers::ai_tool_loop::PendingApproval;
use crate::handlers::conversation_persistence;
use crate::handlers::human_task::{instance_slug, INBOX_TASK_NODE_TYPE, INBOX_WORKSPACE};
use crate::types::{FlowCallbacks, FlowContext};
use serde_json::{json, Value};
use tracing::{info, warn};

/// The task type an agent-initiated approval is filed under.
///
/// Deliberately its OWN slug rather than `approval`: `TaskType` carries any
/// application slug verbatim, and an inbox that can tell "an agent asked you
/// this" from "a workflow step asked you this" can render and route them
/// differently. The runtime treats both the same.
const AGENT_APPROVAL_TASK_TYPE: &str = "agent_approval";

/// Default choices when the agent names none.
fn default_options() -> Value {
    json!([
        { "value": "approve", "label": "Approve", "style": "success" },
        { "value": "reject", "label": "Reject", "style": "danger" },
    ])
}

/// Create the inbox task for an approval the agent asked for.
///
/// Returns the state to park on, or `None` when the request cannot be filed —
/// which the caller must treat as "do not park", because a flow parked on a
/// task that does not exist waits forever.
pub(super) async fn open(
    callbacks: &dyn FlowCallbacks,
    context: &FlowContext,
    session: &mut ChatSessionState,
    config: &ApprovalConfig,
    step_id: &str,
    message_path: &str,
    pending: &PendingApproval,
) -> Option<PendingApprovalState> {
    // The agent may name an assignee; the step's default stands in otherwise.
    // With neither there is nobody to ask, and filing the task into a
    // guessed inbox would be worse than declining.
    let assignee = pending
        .request
        .assignee
        .clone()
        .or_else(|| config.assignee.clone())?;

    let options = if pending.request.options.is_empty() {
        default_options()
    } else {
        Value::Array(pending.request.options.clone())
    };

    let task_path = format!(
        "{}/inbox/task-{}-{}-ap{}",
        assignee.trim_end_matches('/'),
        step_id,
        instance_slug(&context.instance_id),
        session.approval_count,
    );

    let mut props = json!({
        "task_type": AGENT_APPROVAL_TASK_TYPE,
        "title": pending.request.title,
        "assignee": assignee,
        "status": "pending",
        "flow_instance_id": context.instance_id,
        "step_id": step_id,
        "options": options,
        "priority": 3,
        // Provenance: this was the AGENT's judgement that a human was needed,
        // not a gate the flow author placed. An inbox that cannot tell the
        // difference cannot explain to the person why they are being asked.
        "requested_by_agent": session.current_agent,
        "conversation_path": session.conversation_path,
    });
    if let Some(desc) = &pending.request.description {
        props["description"] = Value::String(desc.clone());
    }
    if let Some(secs) = config.due_in_seconds {
        props["due_in_seconds"] = Value::from(secs);
    }

    if let Err(e) = callbacks
        .create_node_in_workspace(INBOX_WORKSPACE, INBOX_TASK_NODE_TYPE, &task_path, props)
        .await
    {
        // Do NOT park. The caller keeps the session running and the agent is
        // told the request failed, so the conversation can carry on rather
        // than hang on a task nobody will ever see.
        warn!(
            task_path = %task_path,
            error = %e,
            "Failed to create agent approval task - not parking"
        );
        return None;
    }

    info!(
        task_path = %task_path,
        title = %pending.request.title,
        "Agent approval task created - parking flow"
    );

    session.approval_count += 1;

    Some(PendingApprovalState {
        tool_call_path: format!("{}/{}", message_path, pending.tool_call_id),
        task_path,
        title: pending.request.title.clone(),
    })
}

/// Answer a parked approval with the human's response, if one has arrived.
///
/// Writes the answer as the pending tool call's result and clears the parked
/// state. Returns `true` when an answer was consumed — the caller must then go
/// straight to an AI turn rather than waiting for a user message, because the
/// conversation is mid-turn and it is the AGENT's move.
pub(super) async fn answer(
    callbacks: &dyn FlowCallbacks,
    context: &mut FlowContext,
    session: &mut ChatSessionState,
    conv_workspace: &str,
) -> bool {
    let Some(pending) = session.pending_approval.clone() else {
        return false;
    };

    // `__human_response` is set by `resume_flow` for a HumanTask wait. Its
    // ABSENCE while parked means this execution is not the resume that carries
    // the answer (a redelivery, a timeout check) — stay parked.
    let Some(response) = context.variables.remove("__human_response") else {
        return false;
    };
    context.variables.remove("__human_response_pending");

    info!(
        task_path = %pending.task_path,
        "Human answered an agent approval - resuming the conversation"
    );

    if let Err(e) = conversation_persistence::persist_tool_result(
        callbacks,
        conv_workspace,
        &pending.tool_call_path,
        &json!({ "approved_by_human": true, "response": response }),
        None,
        None,
    )
    .await
    {
        // The turn continues either way, but the model will not see its
        // question answered — worth a loud log rather than a silent oddity.
        warn!(
            tool_call_path = %pending.tool_call_path,
            error = %e,
            "Failed to persist approval answer as a tool result"
        );
    }

    session.pending_approval = None;
    true
}
