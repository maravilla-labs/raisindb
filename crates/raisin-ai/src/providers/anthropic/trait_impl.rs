//! AIProviderTrait implementation for AnthropicProvider.

use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;

use crate::model_cache::ModelInfo;
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{
    CompletionRequest, CompletionResponse, FunctionCall, Message, Role, StreamChunk, ToolCall,
    Usage,
};

use super::types::*;
use super::AnthropicProvider;

#[async_trait]
impl AIProviderTrait for AnthropicProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        Self::validate_chat_model(&request.model)?;

        let anthropic_request = Self::build_chat_request(&request, false);
        let response = self.send_api_request(&anthropic_request).await?;

        let anthropic_response: AnthropicChatResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        let mut result = parse_response(anthropic_response);

        Self::extract_structured_output(&mut result, request.response_format.as_ref());

        Ok(result)
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "claude-opus-4-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "claude-3-5-sonnet".to_string(),
            "claude-3-5-haiku".to_string(),
            "claude-3-opus".to_string(),
            "claude-3-sonnet".to_string(),
            "claude-3-haiku".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        if let Some(cached) = self.cache.get("anthropic").await {
            return Ok(cached);
        }

        let models = Self::get_known_models();
        self.cache.put("anthropic", models.clone()).await;
        Ok(models)
    }

    async fn generate_embedding(&self, _text: &str, _model: &str) -> Result<Vec<f32>> {
        Err(ProviderError::UnsupportedOperation(
            "Anthropic does not support embeddings. Use OpenAI or other providers for embeddings."
                .to_string(),
        ))
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        use futures::stream::StreamExt;

        Self::validate_chat_model(&request.model)?;

        let anthropic_request = Self::build_chat_request(&request, true);
        let response = self.send_api_request(&anthropic_request).await?;

        let stream = crate::providers::sse::buffered_text_stream(
            response
                .bytes_stream()
                .map(|result| result.map_err(|e| ProviderError::NetworkError(e.to_string()))),
        )
        .flat_map(|result| {
            let chunks: Vec<Result<StreamChunk>> = match result {
                Err(e) => vec![Err(e)],
                Ok(text) => parse_anthropic_sse_events(&text),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

// ── Response parsing ──────────────────────────────────────────────

/// Convert a deserialized Anthropic Messages API response into a
/// `CompletionResponse`.
fn parse_response(anthropic_response: AnthropicChatResponse) -> CompletionResponse {
    let mut text_content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for content in anthropic_response.content {
        match content {
            AnthropicResponseContent::Text { text } => {
                if !text_content.is_empty() {
                    text_content.push('\n');
                }
                text_content.push_str(&text);
            }
            AnthropicResponseContent::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id,
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name,
                        arguments: serde_json::to_string(&input).unwrap_or_default(),
                    },
                    index: None,
                });
            }
        }
    }

    CompletionResponse {
        message: Message {
            role: Role::Assistant,
            content: text_content,
            content_parts: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
            name: None,
        },
        model: anthropic_response.model,
        usage: Some(Usage {
            prompt_tokens: anthropic_response.usage.input_tokens,
            completion_tokens: anthropic_response.usage.output_tokens,
            total_tokens: anthropic_response.usage.input_tokens
                + anthropic_response.usage.output_tokens,
        }),
        stop_reason: anthropic_response.stop_reason,
    }
}

// ── SSE streaming parser ──────────────────────────────────────────

/// Parse SSE events from Anthropic's streaming Messages API response.
///
/// Anthropic uses `event:` lines to indicate the event type, followed by
/// `data:` lines with JSON payloads.
pub(super) fn parse_anthropic_sse_events(text: &str) -> Vec<Result<StreamChunk>> {
    use crate::providers::sse::parse_sse_event_lines;

    parse_sse_event_lines(text)
        .into_iter()
        .filter_map(|sse_event| {
            let event_type = sse_event.event_type.unwrap_or("");
            parse_anthropic_chunk(event_type, sse_event.data)
        })
        .collect()
}

/// Convert a single Anthropic SSE event into an optional `StreamChunk`.
fn parse_anthropic_chunk(event_type: &str, data: &str) -> Option<Result<StreamChunk>> {
    match event_type {
        "content_block_delta" => {
            let event: AnthropicStreamEvent = serde_json::from_str(data).ok()?;
            let delta = event.delta?;

            match delta {
                AnthropicStreamDelta::TextDelta { text } => Some(Ok(StreamChunk {
                    delta: text,
                    tool_calls: None,
                    usage: None,
                    stop_reason: None,
                    model: None,
                })),
                AnthropicStreamDelta::InputJsonDelta { partial_json } => {
                    // Tool call argument delta -- we need the index from the event
                    // to correlate with the tool call started in content_block_start.
                    Some(Ok(StreamChunk {
                        delta: String::new(),
                        tool_calls: Some(vec![ToolCall {
                            id: String::new(),
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: String::new(),
                                arguments: partial_json,
                            },
                            index: event.index,
                        }]),
                        usage: None,
                        stop_reason: None,
                        model: None,
                    }))
                }
                AnthropicStreamDelta::MessageDelta { .. } => None,
            }
        }
        "content_block_start" => {
            let event: AnthropicStreamEvent = serde_json::from_str(data).ok()?;
            let block = event.content_block?;

            match block {
                AnthropicStreamContentBlock::Text { .. } => None,
                AnthropicStreamContentBlock::ToolUse { id, name } => {
                    Some(Ok(StreamChunk {
                        delta: String::new(),
                        tool_calls: Some(vec![ToolCall {
                            id,
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name,
                                arguments: String::new(),
                            },
                            index: event.index,
                        }]),
                        usage: None,
                        stop_reason: None,
                        model: None,
                    }))
                }
            }
        }
        "message_start" => {
            let event: AnthropicStreamEvent = serde_json::from_str(data).ok()?;
            let message = event.message?;

            // Emit the model info and initial usage from message_start
            Some(Ok(StreamChunk {
                delta: String::new(),
                tool_calls: None,
                usage: Some(Usage {
                    prompt_tokens: message.usage.input_tokens,
                    completion_tokens: message.usage.output_tokens,
                    total_tokens: message.usage.input_tokens + message.usage.output_tokens,
                }),
                stop_reason: None,
                model: Some(message.model),
            }))
        }
        "message_delta" => {
            // message_delta carries the stop_reason and final output usage.
            // The data shape is: {"type":"message_delta","delta":{"stop_reason":"..."},"usage":{"output_tokens":N}}
            #[derive(serde::Deserialize)]
            struct MessageDeltaEvent {
                delta: MessageDeltaPayload,
                #[serde(default)]
                usage: Option<AnthropicUsage>,
            }
            #[derive(serde::Deserialize)]
            struct MessageDeltaPayload {
                stop_reason: Option<String>,
            }

            let event: MessageDeltaEvent = serde_json::from_str(data).ok()?;

            Some(Ok(StreamChunk {
                delta: String::new(),
                tool_calls: None,
                usage: event.usage.map(|u| Usage {
                    prompt_tokens: 0,
                    completion_tokens: u.output_tokens,
                    total_tokens: u.output_tokens,
                }),
                stop_reason: event.delta.stop_reason,
                model: None,
            }))
        }
        _ => None,
    }
}
