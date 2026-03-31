// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Message persistence: user messages, assistant responses, and tool results.

use super::types::{AiResponseData, AI_SENDER_ID};
use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowResult};
use serde_json::{json, Value};
use tracing::{debug, warn};

/// Persist a user message as a `raisin:Message` node.
pub async fn persist_user_message(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    conversation_path: &str,
    workspace: &str,
    content: &str,
    sender_id: Option<&str>,
    sender_display_name: Option<&str>,
) -> FlowResult<String> {
    let message_id = nanoid::nanoid!();
    let message_path = format!("{}/{}", conversation_path, message_id);
    let now = chrono::Utc::now().to_rfc3339();

    let sid = sender_id.unwrap_or("user");
    let display = sender_display_name.unwrap_or("You");

    let properties = json!({
        "role": "user",
        "body": {
            "content": content,
            "message_text": content,
        },
        "sender_id": sid,
        "sender_display_name": display,
        "message_type": "chat",
        "status": "delivered",
        "created_at": now,
        "_source": "flow",
    });

    callbacks
        .create_node_in_workspace(workspace, "raisin:Message", &message_path, properties)
        .await?;

    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::message_saved(&message_path, "user", conversation_path),
        )
        .await;

    debug!("Saved user message at: {}", message_path);
    Ok(message_path)
}

/// Persist an assistant response as a `raisin:Message` with child nodes
/// (thoughts, tool calls, cost records).
pub async fn persist_assistant_response(
    callbacks: &dyn FlowCallbacks,
    instance_id: &str,
    conversation_path: &str,
    workspace: &str,
    response: &AiResponseData,
) -> FlowResult<String> {
    let message_id = nanoid::nanoid!();
    let message_path = format!("{}/{}", conversation_path, message_id);
    let now = chrono::Utc::now().to_rfc3339();

    let mut properties = json!({
        "role": "assistant",
        "body": {
            "content": &response.content,
            "message_text": &response.content,
        },
        "sender_id": AI_SENDER_ID,
        "sender_display_name": "AI Assistant",
        "message_type": "chat",
        "status": "delivered",
        "created_at": now,
        "_source": "flow",
    });

    if let Some(ref model) = response.model {
        properties["model"] = Value::String(model.clone());
    }
    properties["finish_reason"] = Value::String(
        match response.finish_reason.as_deref() {
            Some("end_turn") | None => "stop".to_string(),
            Some(reason) => reason.to_string(),
        },
    );

    if let Some(ref usage) = response.usage {
        let mut tokens = serde_json::Map::new();
        if let Some(input) = usage.input_tokens {
            tokens.insert("input".to_string(), json!(input));
        }
        if let Some(output) = usage.output_tokens {
            tokens.insert("output".to_string(), json!(output));
        }
        if !tokens.is_empty() {
            properties["tokens"] = Value::Object(tokens);
        }
    }

    if !response.tool_calls.is_empty() {
        properties["tool_calls"] = serde_json::to_value(
            &response
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "function": { "name": tc.function_name, "arguments": tc.arguments },
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
    }

    callbacks
        .create_node_in_workspace(workspace, "raisin:Message", &message_path, properties)
        .await?;

    persist_child_nodes(callbacks, workspace, &message_path, response).await;

    // Emit MessageSaved event
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::message_saved(&message_path, "assistant", conversation_path),
        )
        .await;

    debug!("Saved assistant response at: {}", message_path);
    Ok(message_path)
}

/// Persist AIThought, AIToolCall, and AICostRecord children under a message.
async fn persist_child_nodes(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    message_path: &str,
    response: &AiResponseData,
) {
    // Create AIThought children
    for (i, thought) in response.thinking.iter().enumerate() {
        if !thought.is_empty() {
            let thought_path = format!("{}/thought-{}", message_path, i);
            let thought_props = json!({
                "content": thought,
                "thought_type": "reasoning",
            });
            if let Err(e) = callbacks
                .create_node_in_workspace(workspace, "raisin:AIThought", &thought_path, thought_props)
                .await
            {
                warn!("Failed to persist thought node {}: {}", thought_path, e);
            }
        }
    }

    // Create AIToolCall children
    for tc in &response.tool_calls {
        let tc_path = format!("{}/{}", message_path, tc.id);
        let tc_props = json!({
            "tool_call_id": tc.id,
            "function_name": tc.function_name,
            "arguments": tc.arguments,
            "status": "pending",
        });
        if let Err(e) = callbacks
            .create_node_in_workspace(workspace, "raisin:AIToolCall", &tc_path, tc_props)
            .await
        {
            warn!("Failed to persist tool call node {}: {}", tc_path, e);
        }
    }

    // Create AICostRecord if usage data available
    if let Some(ref usage) = response.usage {
        let cost_path = format!("{}/cost", message_path);
        let mut cost_props = json!({});
        if let Some(ref model) = usage.model {
            cost_props["model"] = Value::String(model.clone());
        }
        if let Some(ref provider) = usage.provider {
            cost_props["provider"] = Value::String(provider.clone());
        }
        if let Some(tokens) = usage.input_tokens {
            cost_props["input_tokens"] = json!(tokens);
        }
        if let Some(tokens) = usage.output_tokens {
            cost_props["output_tokens"] = json!(tokens);
        }
        if let Err(e) = callbacks
            .create_node_in_workspace(workspace, "raisin:AICostRecord", &cost_path, cost_props)
            .await
        {
            warn!("Failed to persist cost record {}: {}", cost_path, e);
        }
    }
}

/// Persist a tool result under a tool call node.
pub async fn persist_tool_result(
    callbacks: &dyn FlowCallbacks,
    workspace: &str,
    tool_call_path: &str,
    result: &Value,
    error: Option<&str>,
    duration_ms: Option<u64>,
) -> FlowResult<()> {
    let result_path = format!("{}/result", tool_call_path);
    let mut properties = json!({
        "result": result,
        "_source": "flow",
    });
    if let Some(err) = error {
        properties["error"] = Value::String(err.to_string());
    }
    if let Some(dur) = duration_ms {
        properties["duration_ms"] = json!(dur);
    }

    callbacks
        .create_node_in_workspace(workspace, "raisin:AIToolResult", &result_path, properties)
        .await?;

    // Update tool call status
    if let Err(e) = callbacks
        .update_node_in_workspace(
            workspace,
            tool_call_path,
            json!({
                "status": if error.is_some() { "error" } else { "completed" },
            }),
        )
        .await
    {
        warn!(
            "Failed to update tool call status at {}: {}",
            tool_call_path, e
        );
    }

    Ok(())
}
