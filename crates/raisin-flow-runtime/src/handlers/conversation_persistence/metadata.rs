// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversation metadata updates (stats, last message).

use crate::types::{FlowCallbacks, FlowResult};
use serde_json::json;

/// Update conversation statistics (message count, tokens, status).
pub async fn update_conversation_stats(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    conversation_path: &str,
    message_count: u32,
    total_tokens: u64,
    status: &str,
) -> FlowResult<()> {
    callbacks
        .update_node_in_workspace(
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

    callbacks
        .update_node_in_workspace(
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
