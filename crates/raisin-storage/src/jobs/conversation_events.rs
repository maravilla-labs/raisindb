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

//! Conversation event broadcasting system
//!
//! This module provides a broadcast channel-based system for streaming
//! AI conversation events to SSE/WebSocket clients. Each conversation
//! has its own broadcast channel keyed by conversation path.
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_storage::jobs::ConversationEventBroadcaster;
//!
//! let broadcaster = ConversationEventBroadcaster::new();
//!
//! // Subscribe to a conversation's events (returns a receiver)
//! let receiver = broadcaster.subscribe("workspace/conversations/conv-123");
//!
//! // Emit an event (creates channel if needed)
//! broadcaster.emit("workspace/conversations/conv-123", event);
//! ```

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Global conversation event broadcaster instance
static GLOBAL_CONVERSATION_BROADCASTER: Lazy<ConversationEventBroadcaster> =
    Lazy::new(ConversationEventBroadcaster::new);

/// Get the global conversation event broadcaster
///
/// This broadcaster is shared across all AI conversations and streaming clients.
/// AI execution handlers emit events here, and SSE/WebSocket endpoints subscribe here.
pub fn global_conversation_broadcaster() -> &'static ConversationEventBroadcaster {
    &GLOBAL_CONVERSATION_BROADCASTER
}

/// Maximum number of events to buffer per channel
const CHANNEL_CAPACITY: usize = 100;

/// Conversation event for broadcasting
///
/// These events represent real-time updates from an AI conversation,
/// including streaming text, tool calls, and message persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConversationEvent {
    /// Streaming text chunk from AI response
    TextChunk {
        text: String,
        timestamp: String,
    },
    /// AI thinking/reasoning chunk
    ThoughtChunk {
        text: String,
        timestamp: String,
    },
    /// AI tool call has started
    ToolCallStarted {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "functionName")]
        function_name: String,
        arguments: serde_json::Value,
        timestamp: String,
    },
    /// AI tool call has completed
    ToolCallCompleted {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none", rename = "functionName")]
        function_name: Option<String>,
        result: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "durationMs")]
        duration_ms: Option<u64>,
        timestamp: String,
    },
    /// A message was persisted to the conversation node tree
    MessageSaved {
        #[serde(rename = "messagePath")]
        message_path: String,
        role: String,
        #[serde(rename = "conversationPath")]
        conversation_path: String,
        timestamp: String,
    },
    /// AI response is complete for this conversation turn
    Done {
        #[serde(rename = "conversationPath")]
        conversation_path: String,
        /// Final assistant response content (safety net for missed streaming chunks)
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "senderDisplayName")]
        sender_display_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "finishReason")]
        finish_reason: Option<String>,
        timestamp: String,
    },
    /// Conversation turn is paused (e.g., awaiting plan approval).
    /// Unlike Done, this does NOT close the SSE stream.
    Waiting {
        #[serde(rename = "conversationPath")]
        conversation_path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        timestamp: String,
    },
    /// Log message from a backend handler, forwarded to browser console via SSE.
    Log {
        level: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        module: Option<String>,
        timestamp: String,
    },
}

impl ConversationEvent {
    /// Return a string label for the event type (for logging)
    pub fn event_type(&self) -> &'static str {
        match self {
            ConversationEvent::TextChunk { .. } => "text_chunk",
            ConversationEvent::ThoughtChunk { .. } => "thought_chunk",
            ConversationEvent::ToolCallStarted { .. } => "tool_call_started",
            ConversationEvent::ToolCallCompleted { .. } => "tool_call_completed",
            ConversationEvent::MessageSaved { .. } => "message_saved",
            ConversationEvent::Done { .. } => "done",
            ConversationEvent::Waiting { .. } => "waiting",
            ConversationEvent::Log { .. } => "log",
        }
    }
}

/// Broadcaster for conversation events
///
/// Maintains a map of broadcast channels, one per conversation path.
/// Events are emitted to the appropriate channel based on conversation_path.
#[derive(Clone)]
pub struct ConversationEventBroadcaster {
    channels: Arc<DashMap<String, broadcast::Sender<ConversationEvent>>>,
}

impl ConversationEventBroadcaster {
    /// Create a new conversation event broadcaster
    pub fn new() -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Subscribe to events for a specific conversation
    ///
    /// Returns a broadcast receiver that will receive all events for the conversation.
    /// Creates the channel if it doesn't exist.
    pub fn subscribe(&self, conversation_path: &str) -> broadcast::Receiver<ConversationEvent> {
        let sender = self
            .channels
            .entry(conversation_path.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                tx
            });
        sender.subscribe()
    }

    /// Emit an event for a conversation
    ///
    /// If no subscribers exist for the conversation, the event is dropped silently.
    pub fn emit(&self, conversation_path: &str, event: ConversationEvent) {
        if let Some(sender) = self.channels.get(conversation_path) {
            let receiver_count = sender.receiver_count();
            let event_type = event.event_type();
            match sender.send(event) {
                Ok(_) => {
                    tracing::debug!(
                        conversation_path = %conversation_path,
                        event_type = %event_type,
                        receiver_count = receiver_count,
                        "Conversation event emitted"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        conversation_path = %conversation_path,
                        event_type = %event_type,
                        receiver_count = receiver_count,
                        "Conversation event send failed (no receivers)"
                    );
                }
            }
        } else {
            let event_type = event.event_type();
            tracing::warn!(
                conversation_path = %conversation_path,
                event_type = %event_type,
                "No broadcast channel for conversation, event dropped"
            );
        }
    }

    /// Get the number of subscribers for a conversation
    pub fn subscriber_count(&self, conversation_path: &str) -> usize {
        self.channels
            .get(conversation_path)
            .map(|sender| sender.receiver_count())
            .unwrap_or(0)
    }

    /// Remove a channel (call when conversation turn completes and no more events expected)
    pub fn remove_channel(&self, conversation_path: &str) {
        self.channels.remove(conversation_path);
    }

    /// Clean up channels with no subscribers
    pub fn cleanup_empty_channels(&self) {
        self.channels
            .retain(|_, sender| sender.receiver_count() > 0);
    }
}

impl Default for ConversationEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subscribe_and_emit() {
        let broadcaster = ConversationEventBroadcaster::new();
        let mut receiver = broadcaster.subscribe("workspace/conversations/conv-1");

        let event = ConversationEvent::TextChunk {
            text: "Hello".to_string(),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        broadcaster.emit("workspace/conversations/conv-1", event.clone());

        let received = receiver.recv().await.unwrap();
        match received {
            ConversationEvent::TextChunk { text, .. } => {
                assert_eq!(text, "Hello");
            }
            _ => panic!("Expected TextChunk event"),
        }
    }

    #[tokio::test]
    async fn test_done_event() {
        let broadcaster = ConversationEventBroadcaster::new();
        let mut receiver = broadcaster.subscribe("workspace/conversations/conv-2");

        let event = ConversationEvent::Done {
            conversation_path: "workspace/conversations/conv-2".to_string(),
            content: None,
            role: None,
            sender_display_name: None,
            finish_reason: None,
            timestamp: "2025-01-01T00:00:01Z".to_string(),
        };

        broadcaster.emit("workspace/conversations/conv-2", event.clone());

        let received = receiver.recv().await.unwrap();
        match received {
            ConversationEvent::Done {
                conversation_path, ..
            } => {
                assert_eq!(conversation_path, "workspace/conversations/conv-2");
            }
            _ => panic!("Expected Done event"),
        }
    }

    #[test]
    fn test_emit_without_subscribers() {
        let broadcaster = ConversationEventBroadcaster::new();

        // This should not panic
        broadcaster.emit(
            "nonexistent",
            ConversationEvent::TextChunk {
                text: "dropped".to_string(),
                timestamp: "2025-01-01T00:00:00Z".to_string(),
            },
        );
    }

    #[test]
    fn test_subscriber_count() {
        let broadcaster = ConversationEventBroadcaster::new();

        assert_eq!(broadcaster.subscriber_count("test"), 0);

        let _r1 = broadcaster.subscribe("test");
        assert_eq!(broadcaster.subscriber_count("test"), 1);

        let _r2 = broadcaster.subscribe("test");
        assert_eq!(broadcaster.subscriber_count("test"), 2);
    }

    #[test]
    fn test_remove_channel() {
        let broadcaster = ConversationEventBroadcaster::new();

        let _r = broadcaster.subscribe("to-remove");
        assert_eq!(broadcaster.subscriber_count("to-remove"), 1);

        broadcaster.remove_channel("to-remove");
        assert_eq!(broadcaster.subscriber_count("to-remove"), 0);
    }

    #[test]
    fn test_cleanup_empty_channels() {
        let broadcaster = ConversationEventBroadcaster::new();

        // Create a channel with a subscriber, then drop it
        {
            let _r = broadcaster.subscribe("will-be-empty");
        }
        // Subscriber dropped, channel should have 0 receivers
        let _r_active = broadcaster.subscribe("still-active");

        broadcaster.cleanup_empty_channels();

        // "will-be-empty" had 0 receivers so it should be removed
        assert_eq!(broadcaster.subscriber_count("will-be-empty"), 0);
        assert_eq!(broadcaster.subscriber_count("still-active"), 1);
    }

    #[test]
    fn test_event_type() {
        let text = ConversationEvent::TextChunk {
            text: "hi".to_string(),
            timestamp: "t".to_string(),
        };
        assert_eq!(text.event_type(), "text_chunk");

        let thought = ConversationEvent::ThoughtChunk {
            text: "thinking".to_string(),
            timestamp: "t".to_string(),
        };
        assert_eq!(thought.event_type(), "thought_chunk");

        let tool_start = ConversationEvent::ToolCallStarted {
            tool_call_id: "tc-1".to_string(),
            function_name: "search".to_string(),
            arguments: serde_json::json!({}),
            timestamp: "t".to_string(),
        };
        assert_eq!(tool_start.event_type(), "tool_call_started");

        let tool_done = ConversationEvent::ToolCallCompleted {
            tool_call_id: "tc-1".to_string(),
            function_name: Some("search".to_string()),
            result: serde_json::json!({"ok": true}),
            error: None,
            duration_ms: Some(42),
            timestamp: "t".to_string(),
        };
        assert_eq!(tool_done.event_type(), "tool_call_completed");

        let msg = ConversationEvent::MessageSaved {
            message_path: "ws/conv/msg-1".to_string(),
            role: "assistant".to_string(),
            conversation_path: "ws/conv".to_string(),
            timestamp: "t".to_string(),
        };
        assert_eq!(msg.event_type(), "message_saved");

        let done = ConversationEvent::Done {
            conversation_path: "ws/conv".to_string(),
            content: None,
            role: None,
            sender_display_name: None,
            finish_reason: None,
            timestamp: "t".to_string(),
        };
        assert_eq!(done.event_type(), "done");

        let waiting = ConversationEvent::Waiting {
            conversation_path: "ws/conv".to_string(),
            reason: Some("awaiting_plan_approval".to_string()),
            timestamp: "t".to_string(),
        };
        assert_eq!(waiting.event_type(), "waiting");
    }
}
