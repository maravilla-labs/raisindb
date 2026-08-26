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

    /// Maximum retries on transient AI failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,

    /// Base delay between retries in milliseconds (exponential backoff)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_delay_ms: Option<u64>,

    /// Workflow-context injection: "input" | "full" | true (= full).
    /// Appends the workflow context as a JSON block to the agent's task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_context: Option<serde_json::Value>,

    /// Response format: "text", "json_object", or "json_schema"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,

    /// JSON schema for structured output (when response_format = "json_schema")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
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

    /// JSON Schema the agent's final result must take when it ends the session
    /// via the `end_session` control tool. Shapes that tool's `result`
    /// argument, and the payload lands at `steps.<id>.result`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,

    /// Whether the agent may park the flow on a human decision of its own
    /// accord (the `request_approval` control tool).
    #[serde(default)]
    pub approval: ChatApprovalConfig,
}

/// Agent-initiated approval settings for a chat step.
///
/// Off by default: an agent that can park a flow on a person is a capability
/// the flow author grants, never one an agent acquires by being an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatApprovalConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Who an approval goes to when the agent names nobody.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,

    /// Deadline for the human to answer, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_in_seconds: Option<i64>,
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

    /// Conditions under which handoff should occur (raisin-rel expression).
    /// The designer UI saves this as `trigger_condition`.
    #[serde(alias = "trigger_condition", skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,

    /// Phrases that trigger handoff to this agent (designer UI field)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_phrases: Vec<String>,
}

/// Chat session termination configuration.
///
/// Deserializes from either the runtime shape
/// (`{allow_user_end, allow_ai_end, end_keywords}`) or the designer UI
/// shape (`{modes: ["user_request","ai_decision",...], termination_phrases,
/// inactivity_timeout_ms}`).
#[derive(Debug, Clone, Serialize)]
pub struct ChatTerminationConfig {
    /// User can explicitly end the session
    pub allow_user_end: bool,

    /// AI can determine when session is complete
    pub allow_ai_end: bool,

    /// Keywords that trigger session end (e.g., "goodbye", "exit")
    pub end_keywords: Vec<String>,

    /// End the session after this much user inactivity (designer UI field)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactivity_timeout_ms: Option<u64>,
}

impl Default for ChatTerminationConfig {
    fn default() -> Self {
        Self {
            allow_user_end: true,
            allow_ai_end: true,
            end_keywords: Vec::new(),
            inactivity_timeout_ms: None,
        }
    }
}

impl<'de> Deserialize<'de> for ChatTerminationConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            // Runtime shape
            allow_user_end: Option<bool>,
            allow_ai_end: Option<bool>,
            #[serde(default)]
            end_keywords: Vec<String>,
            // Designer UI shape
            #[serde(default)]
            modes: Vec<String>,
            #[serde(default)]
            termination_phrases: Vec<String>,
            inactivity_timeout_ms: Option<u64>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let from_modes = !raw.modes.is_empty();
        Ok(ChatTerminationConfig {
            allow_user_end: raw.allow_user_end.unwrap_or(if from_modes {
                raw.modes.iter().any(|m| m == "user_request")
            } else {
                true
            }),
            allow_ai_end: raw.allow_ai_end.unwrap_or(if from_modes {
                raw.modes.iter().any(|m| m == "ai_decision")
            } else {
                true
            }),
            end_keywords: if raw.end_keywords.is_empty() {
                raw.termination_phrases
            } else {
                raw.end_keywords
            },
            inactivity_timeout_ms: raw.inactivity_timeout_ms,
        })
    }
}
