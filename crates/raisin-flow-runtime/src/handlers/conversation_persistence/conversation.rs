// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversation node creation.

use super::types::{ConversationType, AI_SENDER_ID};
use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowResult};
use serde_json::{json, Value};
use tracing::debug;

/// Ensure a unified conversation node exists at the given path.
///
/// If the node already exists it is left untouched. Otherwise a new
/// `raisin:Conversation` node is created and a `ConversationCreated` event
/// is emitted.
pub async fn ensure_conversation(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    conversation_path: &str,
    workspace: &str,
    conversation_type: ConversationType,
    agent_ref: Option<&str>,
    participants: &[&str],
    participant_details: Option<Value>,
) -> FlowResult<String> {
    // Check if conversation already exists
    if let Ok(Some(_)) = callbacks
        .get_node_in_workspace(workspace, conversation_path)
        .await
    {
        return Ok(conversation_path.to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();

    // Build participant details — if not supplied, derive from agent_ref
    let details = participant_details.unwrap_or_else(|| {
        let mut map = serde_json::Map::new();
        if let Some(r) = agent_ref {
            let display_name = r
                .rsplit('/')
                .next()
                .unwrap_or("AI Assistant")
                .to_string();
            map.insert(
                AI_SENDER_ID.to_string(),
                json!({ "display_name": display_name }),
            );
        }
        Value::Object(map)
    });

    let properties = json!({
        "conversation_type": conversation_type.as_str(),
        "agent_ref": agent_ref,
        "participants": participants,
        "participant_details": details,
        "status": "active",
        "unread_count": 0,
        "updated_at": now,
        "flow_instance_id": instance_id,
        "_source": "flow",
    });

    callbacks
        .create_node_in_workspace(workspace, "raisin:Conversation", conversation_path, properties)
        .await?;

    // Emit ConversationCreated event
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::conversation_created(conversation_path, workspace),
        )
        .await;

    debug!(
        workspace = %workspace,
        path = %conversation_path,
        conversation_type = %conversation_type,
        "Created conversation node"
    );
    Ok(conversation_path.to_string())
}
