// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI and chat configuration types for the designer format
//!
//! Defines configuration structures for AI containers, chat steps,
//! handoff targets, and termination conditions.

use serde::{Deserialize, Serialize};

use super::types::RaisinReference;

/// AI container configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignerAiConfig {
    /// Reference to the agent node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<RaisinReference>,

    /// Tool execution mode
    #[serde(default = "default_tool_mode")]
    pub tool_mode: DesignerToolMode,

    /// Tools to expose as explicit steps (for hybrid mode)
    #[serde(default)]
    pub explicit_tools: Vec<String>,

    /// Maximum iterations for tool call loops
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,

    /// Enable AI thinking/reasoning display
    #[serde(default)]
    pub thinking_enabled: bool,

    /// Reference to existing conversation to continue
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_ref: Option<RaisinReference>,

    /// Error handling behavior
    #[serde(default)]
    pub on_error: DesignerAiErrorBehavior,

    /// Timeout for entire container execution in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Total timeout across all iterations in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_timeout_ms: Option<u64>,
}

fn default_tool_mode() -> DesignerToolMode {
    DesignerToolMode::Auto
}

fn default_max_iterations() -> u32 {
    10
}

/// Tool execution mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignerToolMode {
    /// Agent handles tool calls internally
    #[default]
    Auto,
    /// All tool calls appear as explicit child steps
    Explicit,
    /// Some tools internal, others explicit
    Hybrid,
}

/// AI error handling behavior
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignerAiErrorBehavior {
    /// Stop on error
    #[default]
    Stop,
    /// Continue despite errors
    Continue,
    /// Retry the failed operation
    Retry,
}

/// Chat step configuration for interactive sessions
///
/// Enables multi-turn conversations with sub-agent handoff capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStepConfig {
    /// Reference to the primary AI agent for this chat session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<RaisinReference>,

    /// System prompt to initialize the conversation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Handoff targets - agents that can be delegated to during the session
    #[serde(default)]
    pub handoff_targets: Vec<HandoffTarget>,

    /// Session timeout in milliseconds (how long to wait for user response)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_timeout_ms: Option<u64>,

    /// Maximum number of turns in the conversation
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Termination conditions for the chat session
    #[serde(default)]
    pub termination: ChatTerminationConfig,
}

fn default_max_turns() -> u32 {
    50
}

/// Handoff target configuration for sub-agent delegation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffTarget {
    /// Reference to the handoff agent
    pub agent_ref: RaisinReference,

    /// Description of when to use this agent (for the primary agent's context)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Conditions under which handoff should occur (raisin-rel expression)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// Chat session termination configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatTerminationConfig {
    /// User can explicitly end the session
    #[serde(default = "default_true")]
    pub allow_user_end: bool,

    /// AI can determine when session is complete
    #[serde(default = "default_true")]
    pub allow_ai_end: bool,

    /// Keywords that trigger session end (e.g., "goodbye", "exit")
    #[serde(default)]
    pub end_keywords: Vec<String>,
}

fn default_true() -> bool {
    true
}
