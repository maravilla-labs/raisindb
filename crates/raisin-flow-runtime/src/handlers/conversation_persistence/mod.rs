// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Stateless persistence helpers for unified conversation nodes.
//!
//! All conversations use a single `raisin:Conversation` / `raisin:Message`
//! hierarchy stored under `{user_home}/conversations/`. Child nodes for AI
//! metadata (`raisin:AIThought`, `raisin:AIToolCall`, `raisin:AIToolResult`,
//! `raisin:AICostRecord`) remain unchanged.
//!
//! Every function takes an explicit `workspace` parameter so that user-facing
//! conversations can be stored in `raisin:access_control` while
//! system/AI-container conversations stay in `raisin:system`.

mod conversation;
mod history;
mod messages;
mod metadata;
mod types;

// Re-export types
pub use types::{
    AiResponseData, ConversationType, ToolCallData, UsageData,
    SYSTEM_WORKSPACE, USER_WORKSPACE,
};
pub(crate) use types::AI_SENDER_ID;

// Re-export functions
pub use conversation::ensure_conversation;
pub use history::load_conversation_history;
pub use messages::{persist_assistant_response, persist_tool_result, persist_user_message};
pub use metadata::{update_conversation_last_message, update_conversation_stats};
