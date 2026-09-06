// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversation metadata updates (stats, last message).

use crate::types::{FlowCallbacks, FlowResult};
use serde_json::{json, Value};

/// MERGE a patch into the conversation's properties.
///
/// `update_node_in_workspace` writes the bag it is given as the WHOLE bag, so
/// writing `{last_message, unread_count}` after the first reply erased the
/// conversation's `conversation_type`, `agent_ref`, `participants` and
/// `flow_instance_id` — after which the chat dock could neither name a
/// recipient nor resume the flow ("Conversation recipient not found").
/// Measured 2026-09-03. Read first, then write everything back.
async fn merge_properties(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    conversation_path: &str,
    patch: Value,
) -> FlowResult<()> {
    let mut merged = callbacks
        .get_node_in_workspace(workspace, conversation_path)
        .await
        .ok()
        .flatten()
        .and_then(|n| n.get("properties").cloned())
        .and_then(|p| if p.is_object() { Some(p) } else { None })
        .unwrap_or_else(|| json!({}));
    if let (Some(dst), Some(src)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    callbacks
        .update_node_in_workspace(workspace, conversation_path, merged)
        .await?;
    Ok(())
}

/// Update conversation statistics (message count, tokens, status).
pub async fn update_conversation_stats(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    conversation_path: &str,
    message_count: u32,
    total_tokens: u64,
    status: &str,
) -> FlowResult<()> {
    merge_properties(
        callbacks,
        workspace,
        conversation_path,
        json!({
            "message_count": message_count,
            "total_tokens": total_tokens,
            "status": status,
        }),
    )
    .await?;
    Ok(())
}

/// Update the `last_message` and `unread_count` on a conversation node.
///
/// Keeps the conversation list UI up to date with the latest message
/// preview and unread badge.
pub async fn update_conversation_last_message(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    conversation_path: &str,
    content: &str,
    sender_id: &str,
) -> FlowResult<()> {
    let now = chrono::Utc::now().to_rfc3339();

    merge_properties(
        callbacks,
        workspace,
        conversation_path,
        json!({
            "last_message": {
                "content": content,
                "sender_id": sender_id,
                "created_at": now,
            },
            "unread_count": 1,
            "updated_at": now,
        }),
    )
    .await?;

    Ok(())
}
