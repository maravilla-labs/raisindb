// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversation history loading.

use super::types::AI_SENDER_ID;
use crate::types::{FlowCallbacks, FlowResult};
use serde_json::{json, Value};
use tracing::debug;

/// Load the full conversation history from unified `raisin:Message` children,
/// reconstructing AI-ready messages including tool calls and tool results.
///
/// Handles both the new unified format (`body.content`) and legacy format
/// (`content` at top level) for backward compatibility.
pub async fn load_conversation_history(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    conversation_path: &str,
) -> FlowResult<Vec<Value>> {
    let children = callbacks
        .list_children_in_workspace(workspace, conversation_path)
        .await?;

    let mut messages: Vec<Value> = Vec::new();

    for child in &children {
        let props = child.get("properties").cloned().unwrap_or_default();
        let child_path = child.get("path").and_then(|v| v.as_str()).unwrap_or("");

        let role = extract_role(&props);
        let content = extract_content(&props);

        match role {
            "user" => {
                if content.is_empty() {
                    debug!(
                        "Skipping user message with empty content at '{}'",
                        child_path
                    );
                    continue;
                }
                messages.push(json!({"role": "user", "content": content}));
            }
            "assistant" => {
                append_assistant_message(
                    callbacks,
                    workspace,
                    &props,
                    child_path,
                    content,
                    &mut messages,
                )
                .await;
            }
            "system" => {
                messages.push(json!({"role": "system", "content": content}));
            }
            _ => {} // skip unknown roles
        }
    }

    Ok(messages)
}

/// Determine message role from properties.
fn extract_role(props: &Value) -> &str {
    props
        .get("role")
        .and_then(|v| v.as_str())
        .or_else(|| {
            props.get("sender_id").and_then(|v| v.as_str()).map(|sid| {
                if sid == AI_SENDER_ID {
                    "assistant"
                } else {
                    "user"
                }
            })
        })
        .unwrap_or("")
}

/// Extract message content from unified or legacy format.
fn extract_content<'a>(props: &'a Value) -> &'a str {
    props
        .get("body")
        .and_then(|b| b.get("content"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            props
                .get("body")
                .and_then(|b| b.get("message_text"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| props.get("content").and_then(|v| v.as_str()))
        .unwrap_or("")
}

/// Append an assistant message (with optional tool call results) to the history.
async fn append_assistant_message(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    props: &Value,
    child_path: &str,
    content: &str,
    messages: &mut Vec<Value>,
) {
    let tool_calls = props.get("tool_calls").and_then(|v| v.as_array()).cloned();

    let has_tool_calls = tool_calls
        .as_ref()
        .map(|tcs| !tcs.is_empty())
        .unwrap_or(false);

    if !has_tool_calls {
        messages.push(json!({"role": "assistant", "content": content}));
        return;
    }

    // Assistant message WITH tool calls
    messages.push(json!({
        "role": "assistant",
        "content": content,
        "tool_calls": tool_calls.unwrap(),
    }));

    // Load tool call children to get results
    let msg_children = callbacks
        .list_children_in_workspace(workspace, child_path)
        .await
        .unwrap_or_default();

    for tc_node in &msg_children {
        let tc_props = tc_node.get("properties").cloned().unwrap_or_default();
        let tc_path = tc_node.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let node_type = tc_node
            .get("node_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if node_type != "raisin:AIToolCall" {
            continue;
        }

        let tool_call_id = tc_props
            .get("tool_call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Load result child of this tool call
        let result_children = callbacks
            .list_children_in_workspace(workspace, tc_path)
            .await
            .unwrap_or_default();

        if let Some(result_node) = result_children.first() {
            let result_props = result_node.get("properties").cloned().unwrap_or_default();
            let result_value = result_props.get("result").cloned().unwrap_or(json!(null));
            let result_str = serde_json::to_string(&result_value).unwrap_or_default();

            messages.push(json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": result_str,
            }));
        }
    }
}
