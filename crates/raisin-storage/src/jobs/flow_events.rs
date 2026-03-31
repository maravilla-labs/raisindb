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

//! Flow execution event broadcasting system
//!
//! This module provides a broadcast channel-based system for streaming
//! flow execution events to SSE clients. Each flow instance has its own
//! broadcast channel that clients can subscribe to.
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_storage::jobs::FlowEventBroadcaster;
//!
//! let broadcaster = FlowEventBroadcaster::new();
//!
//! // Subscribe to a flow instance's events (returns a receiver)
//! let receiver = broadcaster.subscribe("instance-123");
//!
//! // Emit an event (creates channel if needed)
//! broadcaster.emit("instance-123", event).await;
//! ```

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Global flow event broadcaster instance
static GLOBAL_FLOW_BROADCASTER: Lazy<FlowEventBroadcaster> = Lazy::new(FlowEventBroadcaster::new);

/// Get the global flow event broadcaster
///
/// This broadcaster is shared across all flow executions and SSE clients.
/// Flow execution handlers emit events here, and SSE endpoints subscribe here.
pub fn global_flow_broadcaster() -> &'static FlowEventBroadcaster {
    &GLOBAL_FLOW_BROADCASTER
}

/// Maximum number of events to buffer per channel
const CHANNEL_CAPACITY: usize = 100;

/// Flow execution event for broadcasting
/// This is a simplified version of FlowExecutionEvent for serialization over SSE
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowEvent {
    /// A step has started execution
    StepStarted {
        node_id: String,
        step_name: Option<String>,
        step_type: String,
        timestamp: String,
    },
    /// A step has completed successfully
    StepCompleted {
        node_id: String,
        output: serde_json::Value,
        duration_ms: u64,
        timestamp: String,
    },
    /// A step has failed
    StepFailed {
        node_id: String,
        error: String,
        duration_ms: u64,
        timestamp: String,
    },
    /// Flow is waiting for external input
    FlowWaiting {
        node_id: String,
        wait_type: String,
        reason: String,
        timestamp: String,
    },
    /// Flow has resumed after waiting
    FlowResumed {
        node_id: String,
        wait_duration_ms: u64,
        timestamp: String,
    },
    /// Flow has completed successfully
    FlowCompleted {
        output: serde_json::Value,
        total_duration_ms: u64,
        timestamp: String,
    },
    /// Flow has failed
    FlowFailed {
        error: String,
        failed_at_node: Option<String>,
        total_duration_ms: u64,
        timestamp: String,
    },
    /// Log message from flow execution
    Log {
        level: String,
        message: String,
        node_id: Option<String>,
        timestamp: String,
    },
    /// Streaming text chunk from AI
    TextChunk { text: String, timestamp: String },
    /// AI tool call started
    ToolCallStarted {
        tool_call_id: String,
        function_name: String,
        arguments: serde_json::Value,
        timestamp: String,
    },
    /// AI tool call completed
    ToolCallCompleted {
        tool_call_id: String,
        result: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        timestamp: String,
    },
    /// AI thinking/reasoning chunk
    ThoughtChunk { text: String, timestamp: String },
    /// AI conversation node was created or resolved
    ConversationCreated {
        conversation_path: String,
        workspace: String,
        timestamp: String,
    },
    /// An AI message was persisted to the node tree
    MessageSaved {
        message_path: String,
        role: String,
        conversation_path: String,
        timestamp: String,
    },
}

impl FlowEvent {
    /// Return a string label for the event type (for logging)
    pub fn event_type(&self) -> &'static str {
        match self {
            FlowEvent::StepStarted { .. } => "step_started",
            FlowEvent::StepCompleted { .. } => "step_completed",
            FlowEvent::StepFailed { .. } => "step_failed",
            FlowEvent::FlowWaiting { .. } => "flow_waiting",
            FlowEvent::FlowResumed { .. } => "flow_resumed",
            FlowEvent::FlowCompleted { .. } => "flow_completed",
            FlowEvent::FlowFailed { .. } => "flow_failed",
            FlowEvent::Log { .. } => "log",
            FlowEvent::TextChunk { .. } => "text_chunk",
            FlowEvent::ToolCallStarted { .. } => "tool_call_started",
            FlowEvent::ToolCallCompleted { .. } => "tool_call_completed",
            FlowEvent::ThoughtChunk { .. } => "thought_chunk",
            FlowEvent::ConversationCreated { .. } => "conversation_created",
            FlowEvent::MessageSaved { .. } => "message_saved",
        }
    }
}

/// Broadcaster for flow execution events
///
/// Maintains a map of broadcast channels, one per flow instance.
/// Events are emitted to the appropriate channel based on instance_id.
#[derive(Clone)]
pub struct FlowEventBroadcaster {
    channels: Arc<DashMap<String, broadcast::Sender<FlowEvent>>>,
}

impl FlowEventBroadcaster {
    /// Create a new flow event broadcaster
    pub fn new() -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Subscribe to events for a specific flow instance
    ///
    /// Returns a broadcast receiver that will receive all events for the instance.
    /// Creates the channel if it doesn't exist.
    pub fn subscribe(&self, instance_id: &str) -> broadcast::Receiver<FlowEvent> {
        let sender = self
            .channels
            .entry(instance_id.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                tx
            });
        sender.subscribe()
    }

    /// Emit an event for a flow instance
    ///
    /// If no subscribers exist for the instance, the event is dropped silently.
    /// This is fine because SSE clients subscribe before flow execution starts.
    pub fn emit(&self, instance_id: &str, event: FlowEvent) {
        if let Some(sender) = self.channels.get(instance_id) {
            let receiver_count = sender.receiver_count();
            let event_type = event.event_type();
            match sender.send(event) {
                Ok(_) => {
                    tracing::debug!(
                        instance_id = %instance_id,
                        event_type = %event_type,
                        receiver_count = receiver_count,
                        "Broadcast event emitted"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        instance_id = %instance_id,
                        event_type = %event_type,
                        receiver_count = receiver_count,
                        "Broadcast event send failed (no receivers)"
                    );
                }
            }
        } else {
            let event_type = event.event_type();
            tracing::warn!(
                instance_id = %instance_id,
                event_type = %event_type,
                "No broadcast channel for instance, event dropped"
            );
        }
    }

    /// Get the number of subscribers for an instance
    pub fn subscriber_count(&self, instance_id: &str) -> usize {
        self.channels
            .get(instance_id)
            .map(|sender| sender.receiver_count())
            .unwrap_or(0)
    }

    /// Remove a channel (call when flow execution completes and no more events expected)
    pub fn remove_channel(&self, instance_id: &str) {
        self.channels.remove(instance_id);
    }

    /// Clean up channels with no subscribers
    pub fn cleanup_empty_channels(&self) {
        self.channels
            .retain(|_, sender| sender.receiver_count() > 0);
    }
}

impl Default for FlowEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subscribe_and_emit() {
        let broadcaster = FlowEventBroadcaster::new();
        let mut receiver = broadcaster.subscribe("test-instance");

        let event = FlowEvent::StepStarted {
            node_id: "step-1".to_string(),
            step_name: Some("Test Step".to_string()),
            step_type: "FunctionStep".to_string(),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        broadcaster.emit("test-instance", event.clone());

        let received = receiver.recv().await.unwrap();
        match received {
            FlowEvent::StepStarted { node_id, .. } => {
                assert_eq!(node_id, "step-1");
            }
            _ => panic!("Expected StepStarted event"),
        }
    }

    #[test]
    fn test_emit_without_subscribers() {
        let broadcaster = FlowEventBroadcaster::new();

        // This should not panic
        broadcaster.emit(
            "nonexistent",
            FlowEvent::StepStarted {
                node_id: "step-1".to_string(),
                step_name: None,
                step_type: "FunctionStep".to_string(),
                timestamp: "2025-01-01T00:00:00Z".to_string(),
            },
        );
    }

    #[test]
    fn test_subscriber_count() {
        let broadcaster = FlowEventBroadcaster::new();

        assert_eq!(broadcaster.subscriber_count("test"), 0);

        let _r1 = broadcaster.subscribe("test");
        assert_eq!(broadcaster.subscriber_count("test"), 1);

        let _r2 = broadcaster.subscribe("test");
        assert_eq!(broadcaster.subscriber_count("test"), 2);
    }
}
