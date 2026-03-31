// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Configuration and state types for the chat step handler.

use crate::types::AiExecutionConfig;
use serde::{Deserialize, Serialize};

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
}
