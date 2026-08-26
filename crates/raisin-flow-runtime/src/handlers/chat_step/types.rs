// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Configuration and state types for the chat step handler.

use crate::types::AiExecutionConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// Re-export the canonical definition so `use types::ConversationType` keeps working.
pub(super) use crate::handlers::conversation_persistence::ConversationType;

/// Chat session configuration parsed from step properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ChatConfig {
    pub agent_ref: Option<String>,
    pub agent_workspace: Option<String>,
    pub system_prompt: Option<String>,
    pub max_turns: u32,
    pub session_timeout_ms: Option<u64>,
    pub handoff_targets: Vec<HandoffTarget>,
    pub termination: TerminationConfig,
    /// Shared AI execution config (retries, timeout, thinking)
    pub execution: AiExecutionConfig,
    /// The type of conversation, determining storage and presentation.
    #[serde(default)]
    pub conversation_type: ConversationType,

    /// JSON Schema the agent's FINAL RESULT must take when it ends the session.
    ///
    /// Lives on the step rather than on the agent because the same agent ends a
    /// triage chat and a drafting chat with different payloads. `None` still
    /// yields a free-form result — see `control_tools`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,

    /// Whether the agent may park the flow on a human decision of its own
    /// accord, and who that decision goes to by default.
    #[serde(default)]
    pub approval: ApprovalConfig,
}

/// Agent-initiated approval settings.
///
/// Default is DISABLED. An agent that can park a flow on a person is a real
/// capability with a real cost, and it must be something the author turned on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct ApprovalConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Who an approval goes to when the agent does not name someone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Deadline for the human to answer, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_in_seconds: Option<i64>,
}

/// A potential handoff target agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct HandoffTarget {
    pub agent_ref: String,
    pub description: Option<String>,
    pub condition: Option<String>,
}

/// Session termination configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct TerminationConfig {
    pub allow_user_end: bool,
    pub allow_ai_end: bool,
    pub end_keywords: Vec<String>,
}

/// Chat session state stored in flow variables.
///
/// Conversation history is loaded from the node tree on each turn,
/// so only metadata is kept here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct ChatSessionState {
    pub session_id: String,
    pub turn_count: u32,
    pub current_agent: Option<String>,
    pub is_complete: bool,
    pub completion_reason: Option<String>,
    pub conversation_path: Option<String>,
    pub total_tokens: u64,

    /// What the agent ended WITH. This is the step's real output — without it
    /// a finished conversation tells the next step only that it happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Set while the flow is parked on an approval the AGENT asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<PendingApprovalState>,

    /// How many approvals this session has asked for. Part of the task path, so
    /// a second approval in the same session cannot alias the first.
    #[serde(default)]
    pub approval_count: u32,
}

/// Where a parked approval left its two anchors.
///
/// `tool_call_path` is the one that matters: the human's answer is written as
/// that tool call's RESULT, which is how the next turn's history rebuild shows
/// the model its question answered. `task_path` exists so the task can be
/// reconciled (and so `WaitInfo::target_path` can point at it for expiry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PendingApprovalState {
    pub tool_call_path: String,
    pub task_path: String,
    pub title: String,
}
