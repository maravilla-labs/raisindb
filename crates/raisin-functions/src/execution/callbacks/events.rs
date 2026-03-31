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

//! Event emission callbacks for function execution.
//!
//! These callbacks implement the `raisin.events.*` API available to JavaScript functions.

use std::sync::Arc;

use raisin_storage::jobs::{global_conversation_broadcaster, ConversationEvent};
use serde_json::Value;

use crate::api::EmitEventCallback;

/// Create emit_event callback: `raisin.events.emit(eventType, data)`
///
/// Routes `conversation:*` events to the global conversation broadcaster so
/// SSE/WebSocket clients receive them in real-time. Other event types are
/// logged but otherwise dropped (no general event bus yet).
pub fn create_emit_event() -> EmitEventCallback {
    Arc::new(move |event_type: String, data: Value| {
        Box::pin(async move {
            if event_type.starts_with("conversation:") {
                let channel = data
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.is_empty())
                    .map(String::from);
                let conversation_path = data
                    .get("conversationPath")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.is_empty())
                    .map(String::from);

                let routing_key = channel.or(conversation_path);

                if routing_key.is_none() {
                    tracing::warn!(
                        event_type = %event_type,
                        "conversation:* event missing both 'channel' and 'conversationPath' fields"
                    );
                    return Ok(());
                }

                match serde_json::from_value::<ConversationEvent>(data.clone()) {
                    Ok(event) => {
                        tracing::debug!(
                            target: "conversation_events",
                            event_type = %event_type,
                            channel = ?routing_key,
                            "Emitting conversation event"
                        );
                        global_conversation_broadcaster()
                            .emit(routing_key.as_ref().unwrap(), event);
                    }
                    Err(e) => {
                        tracing::warn!(
                            event_type = %event_type,
                            error = %e,
                            "Failed to parse conversation event from emit data"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    event_type = %event_type,
                    "Function emitted event (non-conversation, no-op)"
                );
            }
            Ok(())
        })
    })
}
