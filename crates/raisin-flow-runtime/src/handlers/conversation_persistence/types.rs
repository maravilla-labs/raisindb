// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Types and constants for unified conversation persistence.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default workspace for system-owned conversations.
pub const SYSTEM_WORKSPACE: &str = "raisin:system";

/// Workspace for user-owned conversations (access-controlled).
pub const USER_WORKSPACE: &str = "raisin:access_control";

/// Sender ID used for AI-generated messages.
pub(crate) const AI_SENDER_ID: &str = "ai-assistant";

// ---------------------------------------------------------------------------
// ConversationType
// ---------------------------------------------------------------------------

/// The type of conversation, serialized as snake_case strings.
///
/// This is the single canonical definition — used by both
/// `conversation_persistence` and `chat_step`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationType {
    /// User-initiated AI chat
    #[default]
    AiChat,
    /// Flow-initiated chat with user
    FlowChat,
    /// User-to-user direct message
    DirectMessage,
}

impl ConversationType {
    /// Returns the snake_case string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AiChat => "ai_chat",
            Self::FlowChat => "flow_chat",
            Self::DirectMessage => "direct_message",
        }
    }
}

impl std::fmt::Display for ConversationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Data for an AI response to persist.
pub struct AiResponseData {
    pub content: String,
    pub model: Option<String>,
    pub finish_reason: Option<String>,
    pub tool_calls: Vec<ToolCallData>,
    pub usage: Option<UsageData>,
    pub thinking: Vec<String>,
}

/// A single tool call extracted from an AI response.
pub struct ToolCallData {
    pub id: String,
    pub function_name: String,
    pub arguments: Value,
    pub error: Option<String>,
}

/// Token usage and cost metadata.
pub struct UsageData {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub model: Option<String>,
    pub provider: Option<String>,
}
