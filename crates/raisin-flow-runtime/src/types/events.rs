// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow execution events for real-time step-level tracking
//!
//! These events are emitted during flow execution to enable:
//! - Real-time canvas highlighting (show which step is executing)
//! - Timeline visualization (step-by-step progress, variables, logs)
//! - Execution monitoring and debugging

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Events emitted during flow execution for real-time tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowExecutionEvent {
    /// A step has started execution
    StepStarted {
        /// The node ID from the flow definition
        node_id: String,
        /// Human-readable step name (if configured)
        step_name: Option<String>,
        /// Step type (e.g., "function", "decision", "human_task")
        step_type: String,
        /// When the step started
        timestamp: DateTime<Utc>,
    },

    /// A step has completed successfully
    StepCompleted {
        /// The node ID from the flow definition
        node_id: String,
        /// Output produced by the step
        output: Value,
        /// Execution duration in milliseconds
        duration_ms: u64,
        /// When the step completed
        timestamp: DateTime<Utc>,
    },

    /// A step has failed
    StepFailed {
        /// The node ID from the flow definition
        node_id: String,
        /// Error message
        error: String,
        /// Execution duration before failure in milliseconds
        duration_ms: u64,
        /// When the step failed
        timestamp: DateTime<Utc>,
    },

    /// Flow is waiting for external input (human task, approval, etc.)
    FlowWaiting {
        /// The node ID where flow is waiting
        node_id: String,
        /// Type of wait (e.g., "human_task", "approval", "external_event")
        wait_type: String,
        /// Human-readable reason for waiting
        reason: String,
        /// When the wait started
        timestamp: DateTime<Utc>,
    },

    /// Flow has resumed after waiting
    FlowResumed {
        /// The node ID that was waiting
        node_id: String,
        /// How long the flow was waiting in milliseconds
        wait_duration_ms: u64,
        /// When the flow resumed
        timestamp: DateTime<Utc>,
    },

    /// Flow has completed successfully
    FlowCompleted {
        /// Final output from the flow
        output: Value,
        /// Total execution duration in milliseconds
        total_duration_ms: u64,
        /// When the flow completed
        timestamp: DateTime<Utc>,
    },

    /// Flow has failed
    FlowFailed {
        /// Error message
        error: String,
        /// Node ID where failure occurred (if applicable)
        failed_at_node: Option<String>,
        /// Total duration before failure in milliseconds
        total_duration_ms: u64,
        /// When the flow failed
        timestamp: DateTime<Utc>,
    },

    /// Partial text content from AI streaming
    TextChunk {
        /// The text fragment
        text: String,
        /// When the chunk was received
        timestamp: DateTime<Utc>,
    },

    /// An AI tool call has started
    ToolCallStarted {
        /// Unique ID for this tool call
        tool_call_id: String,
        /// Name of the function being called
        function_name: String,
        /// Arguments passed to the function
        arguments: Value,
        /// When the tool call started
        timestamp: DateTime<Utc>,
    },

    /// An AI tool call has completed
    ToolCallCompleted {
        /// Unique ID for this tool call
        tool_call_id: String,
        /// Result from the tool execution
        result: Value,
        /// Error message if the tool call failed
        error: Option<String>,
        /// Execution duration in milliseconds
        duration_ms: Option<u64>,
        /// When the tool call completed
        timestamp: DateTime<Utc>,
    },

    /// Partial thinking/reasoning content from AI streaming
    ThoughtChunk {
        /// The thought fragment
        text: String,
        /// When the chunk was received
        timestamp: DateTime<Utc>,
    },

    /// AI conversation node was created or resolved
    ConversationCreated {
        /// Path to the conversation node
        conversation_path: String,
        /// Workspace where the conversation lives
        workspace: String,
        /// When the conversation was created
        timestamp: DateTime<Utc>,
    },

    /// An AI message was persisted to the node tree
    MessageSaved {
        /// Path to the message node
        message_path: String,
        /// Message role (user, assistant, system, tool)
        role: String,
        /// Path to the parent conversation node
        conversation_path: String,
        /// When the message was saved
        timestamp: DateTime<Utc>,
    },

    /// Log message from flow execution
    Log {
        /// Log level (debug, info, warn, error)
        level: String,
        /// Log message
        message: String,
        /// Optional node context
        node_id: Option<String>,
        /// When the log was emitted
        timestamp: DateTime<Utc>,
    },
}

impl FlowExecutionEvent {
    /// Create a StepStarted event
    pub fn step_started(
        node_id: impl Into<String>,
        step_name: Option<String>,
        step_type: impl Into<String>,
    ) -> Self {
        Self::StepStarted {
            node_id: node_id.into(),
            step_name,
            step_type: step_type.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a StepCompleted event
    pub fn step_completed(node_id: impl Into<String>, output: Value, duration_ms: u64) -> Self {
        Self::StepCompleted {
            node_id: node_id.into(),
            output,
            duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a StepFailed event
    pub fn step_failed(
        node_id: impl Into<String>,
        error: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self::StepFailed {
            node_id: node_id.into(),
            error: error.into(),
            duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a FlowWaiting event
    pub fn flow_waiting(
        node_id: impl Into<String>,
        wait_type: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::FlowWaiting {
            node_id: node_id.into(),
            wait_type: wait_type.into(),
            reason: reason.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a FlowResumed event
    pub fn flow_resumed(node_id: impl Into<String>, wait_duration_ms: u64) -> Self {
        Self::FlowResumed {
            node_id: node_id.into(),
            wait_duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a FlowCompleted event
    pub fn flow_completed(output: Value, total_duration_ms: u64) -> Self {
        Self::FlowCompleted {
            output,
            total_duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a FlowFailed event
    pub fn flow_failed(
        error: impl Into<String>,
        failed_at_node: Option<String>,
        total_duration_ms: u64,
    ) -> Self {
        Self::FlowFailed {
            error: error.into(),
            failed_at_node,
            total_duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a TextChunk event
    pub fn text_chunk(text: impl Into<String>) -> Self {
        Self::TextChunk {
            text: text.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a ToolCallStarted event
    pub fn tool_call_started(
        tool_call_id: impl Into<String>,
        function_name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        Self::ToolCallStarted {
            tool_call_id: tool_call_id.into(),
            function_name: function_name.into(),
            arguments,
            timestamp: Utc::now(),
        }
    }

    /// Create a ToolCallCompleted event
    pub fn tool_call_completed(
        tool_call_id: impl Into<String>,
        result: Value,
        error: Option<String>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self::ToolCallCompleted {
            tool_call_id: tool_call_id.into(),
            result,
            error,
            duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a ThoughtChunk event
    pub fn thought_chunk(text: impl Into<String>) -> Self {
        Self::ThoughtChunk {
            text: text.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a ConversationCreated event
    pub fn conversation_created(
        conversation_path: impl Into<String>,
        workspace: impl Into<String>,
    ) -> Self {
        Self::ConversationCreated {
            conversation_path: conversation_path.into(),
            workspace: workspace.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a MessageSaved event
    pub fn message_saved(
        message_path: impl Into<String>,
        role: impl Into<String>,
        conversation_path: impl Into<String>,
    ) -> Self {
        Self::MessageSaved {
            message_path: message_path.into(),
            role: role.into(),
            conversation_path: conversation_path.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a Log event
    pub fn log(
        level: impl Into<String>,
        message: impl Into<String>,
        node_id: Option<String>,
    ) -> Self {
        Self::Log {
            level: level.into(),
            message: message.into(),
            node_id,
            timestamp: Utc::now(),
        }
    }
}
