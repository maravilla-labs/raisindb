// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Trigger and resume event type definitions
//!
//! Defines the events that cause flows to start (`FlowTriggerEvent`) or
//! resume from a waiting state (`FlowResumeReason`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Events that can trigger or resume a flow
///
/// These events are matched against flow definitions to determine which
/// flows should be started or resumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowTriggerEvent {
    /// A node was created, updated, deleted, or published
    NodeEvent {
        /// Type of event (Created, Updated, Deleted, Published)
        event_type: String,
        /// Node ID that triggered the event
        node_id: String,
        /// Node type of the affected node
        node_type: String,
        /// Path to the affected node
        node_path: String,
        /// Node properties at the time of the event
        properties: serde_json::Value,
        /// Timestamp when the event occurred
        timestamp: DateTime<Utc>,
    },

    /// An AI tool call completed with a result
    ToolResult {
        /// Tool call ID that uniquely identifies this call
        tool_call_id: String,
        /// Name of the tool that was called
        tool_name: String,
        /// Result data returned by the tool
        result: serde_json::Value,
        /// Whether the tool execution was successful
        success: bool,
        /// Error message if the tool failed
        error: Option<String>,
        /// Timestamp when the tool completed
        timestamp: DateTime<Utc>,
    },

    /// A human task was completed
    HumanTaskCompleted {
        /// Task ID that was completed
        task_id: String,
        /// Type of task (approval, form, input, etc.)
        task_type: String,
        /// Response data from the human
        response: serde_json::Value,
        /// Who completed the task
        completed_by: String,
        /// Timestamp when the task was completed
        timestamp: DateTime<Utc>,
    },

    /// A scheduled time was reached
    ScheduledTime {
        /// Schedule ID that triggered
        schedule_id: String,
        /// When the schedule was supposed to trigger
        scheduled_time: DateTime<Utc>,
        /// Actual time when the trigger fired
        actual_time: DateTime<Utc>,
    },

    /// A custom application event occurred
    CustomEvent {
        /// Event name/type
        event_name: String,
        /// Event payload
        payload: serde_json::Value,
        /// Timestamp when the event occurred
        timestamp: DateTime<Utc>,
    },

    /// Manual execution triggered via API
    Manual {
        /// Who triggered the manual execution
        actor: String,
        /// Home path of the actor in raisin:access_control (e.g., "/users/internal/admin")
        actor_home: Option<String>,
        /// Timestamp when the manual trigger was fired
        timestamp: DateTime<Utc>,
    },
}

impl FlowTriggerEvent {
    /// Get a unique identifier for this event
    ///
    /// This is used for deduplication and tracking
    pub fn event_id(&self) -> String {
        match self {
            FlowTriggerEvent::NodeEvent {
                node_id,
                event_type,
                timestamp,
                ..
            } => format!("node:{}:{}:{}", node_id, event_type, timestamp.timestamp()),
            FlowTriggerEvent::ToolResult { tool_call_id, .. } => {
                format!("tool:{}", tool_call_id)
            }
            FlowTriggerEvent::HumanTaskCompleted { task_id, .. } => {
                format!("human_task:{}", task_id)
            }
            FlowTriggerEvent::ScheduledTime { schedule_id, .. } => {
                format!("schedule:{}", schedule_id)
            }
            FlowTriggerEvent::CustomEvent {
                event_name,
                timestamp,
                ..
            } => format!("custom:{}:{}", event_name, timestamp.timestamp()),
            FlowTriggerEvent::Manual {
                actor, timestamp, ..
            } => {
                format!("manual:{}:{}", actor, timestamp.timestamp())
            }
        }
    }

    /// Get the timestamp when this event occurred
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            FlowTriggerEvent::NodeEvent { timestamp, .. } => *timestamp,
            FlowTriggerEvent::ToolResult { timestamp, .. } => *timestamp,
            FlowTriggerEvent::HumanTaskCompleted { timestamp, .. } => *timestamp,
            FlowTriggerEvent::ScheduledTime { actual_time, .. } => *actual_time,
            FlowTriggerEvent::CustomEvent { timestamp, .. } => *timestamp,
            FlowTriggerEvent::Manual { timestamp, .. } => *timestamp,
        }
    }

    /// Get a human-readable description of this event
    pub fn description(&self) -> String {
        match self {
            FlowTriggerEvent::NodeEvent {
                event_type,
                node_path,
                ..
            } => format!("Node {} at {}", event_type, node_path),
            FlowTriggerEvent::ToolResult {
                tool_name, success, ..
            } => {
                if *success {
                    format!("Tool '{}' completed successfully", tool_name)
                } else {
                    format!("Tool '{}' failed", tool_name)
                }
            }
            FlowTriggerEvent::HumanTaskCompleted {
                task_type,
                completed_by,
                ..
            } => format!("Human task '{}' completed by {}", task_type, completed_by),
            FlowTriggerEvent::ScheduledTime { scheduled_time, .. } => {
                format!("Scheduled trigger at {}", scheduled_time)
            }
            FlowTriggerEvent::CustomEvent { event_name, .. } => {
                format!("Custom event '{}'", event_name)
            }
            FlowTriggerEvent::Manual { actor, .. } => {
                format!("Manual execution by {}", actor)
            }
        }
    }
}

/// Reasons why a flow execution is being resumed
///
/// This helps the runtime understand what data is available and how to
/// continue execution from a waiting state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowResumeReason {
    /// Resuming because a tool result arrived
    ToolResultArrived {
        /// The tool call ID that was waiting for
        tool_call_id: String,
        /// The result data
        result: serde_json::Value,
        /// Whether the tool succeeded
        success: bool,
    },

    /// Resuming because a human task was completed
    HumanTaskCompleted {
        /// The task ID that was completed
        task_id: String,
        /// The response from the human
        response: serde_json::Value,
    },

    /// Resuming after a scheduled delay
    ScheduledDelay {
        /// When the delay was supposed to end
        scheduled_time: DateTime<Utc>,
    },

    /// Resuming after an error retry backoff
    RetryAfterError {
        /// The error that occurred
        previous_error: String,
        /// Which retry attempt this is (1-based)
        retry_attempt: u32,
    },

    /// Resuming after parallel branches completed
    ParallelJoinCompleted {
        /// Results from all parallel branches
        branch_results: Vec<serde_json::Value>,
    },

    /// Resuming from manual intervention
    ManualResume {
        /// Who manually resumed the flow
        resumed_by: String,
        /// Optional override data
        override_data: Option<serde_json::Value>,
    },
}

impl FlowResumeReason {
    /// Get a human-readable description of this resume reason
    pub fn description(&self) -> String {
        match self {
            FlowResumeReason::ToolResultArrived {
                tool_call_id,
                success,
                ..
            } => {
                if *success {
                    format!("Tool call '{}' completed successfully", tool_call_id)
                } else {
                    format!("Tool call '{}' failed", tool_call_id)
                }
            }
            FlowResumeReason::HumanTaskCompleted { task_id, .. } => {
                format!("Human task '{}' was completed", task_id)
            }
            FlowResumeReason::ScheduledDelay { scheduled_time } => {
                format!("Scheduled delay reached at {}", scheduled_time)
            }
            FlowResumeReason::RetryAfterError { retry_attempt, .. } => {
                format!("Retry attempt {}", retry_attempt)
            }
            FlowResumeReason::ParallelJoinCompleted { branch_results } => {
                format!(
                    "Parallel join completed ({} branches)",
                    branch_results.len()
                )
            }
            FlowResumeReason::ManualResume { resumed_by, .. } => {
                format!("Manually resumed by {}", resumed_by)
            }
        }
    }

    /// Check if this resume includes error information
    pub fn is_error_retry(&self) -> bool {
        matches!(self, FlowResumeReason::RetryAfterError { .. })
    }

    /// Get the retry attempt number if this is an error retry
    pub fn retry_attempt(&self) -> Option<u32> {
        match self {
            FlowResumeReason::RetryAfterError { retry_attempt, .. } => Some(*retry_attempt),
            _ => None,
        }
    }
}
