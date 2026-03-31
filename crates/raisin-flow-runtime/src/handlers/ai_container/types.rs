// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//\! Types for AI container step handler
//\!
//\! Contains data structures for managing AI container state, messages,
//\! tool calls, and tool results within the flow runtime.

use raisin_ai::types::{FunctionCall, Message, Role, ToolCall as AiToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Resolved agent reference with workspace info
///
/// When agent_ref is stored as a Reference object (raisin:ref format),
/// this struct holds the parsed workspace and path.
#[derive(Debug, Clone)]
pub struct AgentReference {
    /// Node ID (optional, from raisin:ref)
    pub id: Option<String>,
    /// Workspace where the agent is stored (e.g., "functions", "ai")
    pub workspace: String,
    /// Path to the agent node
    pub path: String,
}

/// State for AI container execution
///
/// Tracks metadata, pending tool calls, and completion status.
/// Conversation history is loaded from the node tree on each turn —
/// it is NOT stored in state to avoid unbounded memory growth.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiContainerState {
    /// Current iteration count
    #[serde(default)]
    pub iteration: u32,

    /// Pending tool calls to execute
    #[serde(default)]
    pub pending_tool_calls: Vec<ToolCall>,

    /// Tool results collected
    #[serde(default)]
    pub tool_results: Vec<ToolResult>,

    /// Whether the agent has finished
    #[serde(default)]
    pub completed: bool,

    /// Final response from agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_response: Option<String>,

    /// Epoch millis when execution started (for total timeout enforcement)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
}

/// Helper functions for AI container state management
impl AiContainerState {
    /// Add a tool result to the state
    pub fn add_tool_result(&mut self, result: ToolResult) {
        // Remove from pending if it was explicit
        self.pending_tool_calls
            .retain(|tc| tc.id != result.tool_call_id);
        self.tool_results.push(result);
    }

    /// Mark the container as completed with final response
    pub fn complete(&mut self, response: String) {
        self.completed = true;
        self.final_response = Some(response);
    }

    /// Set pending tool calls for explicit execution
    pub fn set_pending_tools(&mut self, tool_calls: Vec<ToolCall>) {
        self.pending_tool_calls = tool_calls;
    }
}

/// AI message in conversation
///
/// Represents a single message in the AI conversation history.
/// This is similar to raisin_ai::types::Message but with additional
/// flow-specific metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMessage {
    /// Message role
    pub role: MessageRole,

    /// Message content
    pub content: String,

    /// Tool calls (for assistant messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Tool call ID (for tool response messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl From<AiMessage> for Message {
    fn from(msg: AiMessage) -> Self {
        Message {
            role: msg.role.into(),
            content: msg.content,
            content_parts: None,
            tool_calls: msg.tool_calls.map(|calls| {
                calls
                    .into_iter()
                    .map(|tc| AiToolCall {
                        id: tc.id,
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: tc.name,
                            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        },
                        index: None,
                    })
                    .collect()
            }),
            tool_call_id: msg.tool_call_id,
            name: None,
        }
    }
}

/// Message role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// System message
    System,
    /// User message
    User,
    /// Assistant message
    Assistant,
    /// Tool response message
    Tool,
}

impl From<MessageRole> for Role {
    fn from(role: MessageRole) -> Self {
        match role {
            MessageRole::System => Role::System,
            MessageRole::User => Role::User,
            MessageRole::Assistant => Role::Assistant,
            MessageRole::Tool => Role::Tool,
        }
    }
}

impl From<Role> for MessageRole {
    fn from(role: Role) -> Self {
        match role {
            Role::System => MessageRole::System,
            Role::User => MessageRole::User,
            Role::Assistant => MessageRole::Assistant,
            Role::Tool => MessageRole::Tool,
        }
    }
}

/// Tool call from AI
///
/// Represents a request from the AI to execute a tool/function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call
    pub id: String,

    /// Name of the tool/function to call
    pub name: String,

    /// Arguments for the tool call
    pub arguments: Value,
}

/// Result of a tool execution
///
/// Contains the result or error from executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// ID of the tool call this result is for
    pub tool_call_id: String,

    /// Name of the tool that was executed
    pub name: String,

    /// Result value (if successful)
    pub result: Value,

    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of processing tool calls
///
/// Indicates how tool calls should be handled based on the tool mode.
#[derive(Debug)]
pub enum ToolProcessingResult {
    /// No tool calls to process
    NoTools,

    /// Execute all tools automatically
    AutoExecute(Vec<ToolCall>),

    /// Wait for explicit tool step completion
    ExplicitWait(Vec<ToolCall>),

    /// Mixed mode: some auto, some explicit
    Mixed {
        /// Tools to execute automatically
        auto_tools: Vec<ToolCall>,
        /// Tools that need explicit steps
        explicit_tools: Vec<ToolCall>,
    },
}
