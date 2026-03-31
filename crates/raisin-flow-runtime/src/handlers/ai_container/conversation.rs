// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversation management for AI containers
//!
//! Handles loading conversation history, saving assistant responses,
//! and managing conversation state within the flow context.

use super::types::{AiMessage, MessageRole, ToolCall};
use crate::handlers::conversation_persistence::{
    self, AiResponseData, ToolCallData, UsageData,
};
use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowResult};
use serde_json::Value;
use tracing::{debug, error};

use super::AiContainerHandler;

impl AiContainerHandler {
    /// Load conversation history from the node tree
    ///
    /// Loads all AIMessage children of the conversation node and converts them
    /// to `AiMessage` values. Each child is expected to have `properties` with
    /// at least `role` and `content` fields.
    pub(super) async fn load_conversation_history(
        &self,
        conversation_path: &str,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<Vec<AiMessage>> {
        debug!("Loading conversation history from: {}", conversation_path);

        let children = callbacks.list_children(conversation_path).await?;
        if children.is_empty() {
            debug!("No conversation history found");
            return Ok(Vec::new());
        }

        let mut messages = Vec::with_capacity(children.len());

        for child in &children {
            let props = child
                .get("properties")
                .unwrap_or(child);

            let role_str = props
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user");

            let role = match role_str {
                "assistant" => MessageRole::Assistant,
                "system" => MessageRole::System,
                "tool" => MessageRole::Tool,
                _ => MessageRole::User,
            };

            let content = props
                .get("content")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    props
                        .get("body")
                        .and_then(|b| b.get("content"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("")
                .to_string();

            let tool_calls = props
                .get("tool_calls")
                .and_then(|v| serde_json::from_value::<Vec<ToolCall>>(v.clone()).ok());

            let tool_call_id = props
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .map(String::from);

            messages.push(AiMessage {
                role,
                content,
                tool_calls,
                tool_call_id,
            });
        }

        debug!("Loaded {} messages from conversation history", messages.len());
        Ok(messages)
    }

    /// Save an assistant response to the conversation tree.
    ///
    /// Creates a new `raisin:Message` node (plus child `AIThought`,
    /// `AIToolCall`, `AICostRecord` nodes) under the conversation using
    /// the shared persistence helpers.
    ///
    /// Returns the message path of the created node.
    pub(super) async fn save_assistant_response(
        &self,
        context: &FlowContext,
        callbacks: &dyn FlowCallbacks,
        content: &str,
        tool_calls: &Option<Vec<ToolCall>>,
        ai_response: &Value,
    ) -> FlowResult<String> {
        let conversation_path = self.get_conversation_path(context)?;

        debug!(
            "Saving assistant response to conversation: {}",
            conversation_path
        );

        // Ensure conversation node exists
        // AI container is system context — always uses raisin:system workspace
        conversation_persistence::ensure_conversation(
            callbacks,
            &context.instance_id,
            &conversation_path,
            conversation_persistence::SYSTEM_WORKSPACE,
            conversation_persistence::ConversationType::AiChat,
            None, // agent_ref
            &[],
            None,
        )
        .await?;

        // Extract thinking blocks
        let thinking = ai_response
            .get("thinking")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default();

        // Extract usage data
        let usage = {
            let usage_obj = ai_response.get("usage");
            let input_tokens = usage_obj
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .or_else(|| {
                    ai_response.get("input_tokens").and_then(|v| v.as_u64())
                });
            let output_tokens = usage_obj
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .or_else(|| {
                    ai_response.get("output_tokens").and_then(|v| v.as_u64())
                });
            let model = ai_response
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from);
            let provider = ai_response
                .get("provider")
                .and_then(|v| v.as_str())
                .map(String::from);

            if input_tokens.is_some() || output_tokens.is_some() {
                Some(UsageData {
                    input_tokens,
                    output_tokens,
                    model,
                    provider,
                })
            } else {
                None
            }
        };

        // Build tool call data
        let tc_data: Vec<ToolCallData> = tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .map(|tc| ToolCallData {
                        id: tc.id.clone(),
                        function_name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                        error: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let response_data = AiResponseData {
            content: content.to_string(),
            model: ai_response
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from),
            finish_reason: ai_response
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .map(String::from),
            tool_calls: tc_data,
            usage,
            thinking,
        };

        let message_path = conversation_persistence::persist_assistant_response(
            callbacks,
            &context.instance_id,
            &conversation_path,
            conversation_persistence::SYSTEM_WORKSPACE,
            &response_data,
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to save assistant response: {}", e)))?;

        debug!("Saved assistant response at: {}", message_path);
        Ok(message_path)
    }

    /// Get conversation path from flow context
    pub(crate) fn get_conversation_path(&self, context: &FlowContext) -> FlowResult<String> {
        // Get message path from trigger info or flow input
        let message_path = context
            .trigger_info
            .as_ref()
            .and_then(|t| t.node_path.clone())
            .or_else(|| {
                context
                    .input
                    .get("event")
                    .and_then(|e| e.get("node_path"))
                    .and_then(|p| p.as_str())
                    .map(String::from)
            })
            .ok_or_else(|| {
                FlowError::MissingProperty(
                    "Cannot get conversation path: message path not found".to_string(),
                )
            })?;

        // Derive conversation path (parent of message)
        let conversation_path = message_path
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .ok_or_else(|| {
                FlowError::MissingProperty(format!(
                    "Cannot get conversation path: invalid message path: {}",
                    message_path
                ))
            })?
            .to_string();

        Ok(conversation_path)
    }

    /// Append the triggering user message to a messages list.
    pub(super) fn init_user_message_to(
        &self,
        context: &FlowContext,
        messages: &mut Vec<AiMessage>,
    ) {
        // Get user message content from flow input
        if let Some(content) = context
            .input
            .get("event")
            .and_then(|e| e.get("node_data"))
            .and_then(|n| n.get("properties"))
            .and_then(|p| p.get("content"))
            .and_then(|c| c.as_str())
        {
            debug!("Adding user message: {}", content);
            messages.push(AiMessage {
                role: MessageRole::User,
                content: content.to_string(),
                tool_calls: None,
                tool_call_id: None,
            });
        }
    }
}
