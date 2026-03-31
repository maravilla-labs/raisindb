//! AIProviderTrait implementation for OpenAI.

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
use super::OpenAIProvider;

#[async_trait]
impl AIProviderTrait for OpenAIProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        Self::validate_chat_model(&request.model)?;

        let openai_request = Self::build_responses_request(&request, false);
        let url = format!("{}/responses", self.base_url);
        let response = self.send_api_request(&url, &openai_request).await?;

        let openai_response: OpenAIResponsesResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        parse_responses_response(openai_response)
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "gpt-4.1".to_string(),
            "gpt-4o".to_string(),
            "gpt-4-turbo".to_string(),
            "gpt-4".to_string(),
            "gpt-3.5-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            "o1-preview".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        if let Some(cached) = self.cache().get("openai").await {
            return Ok(cached);
        }

        let models = self.fetch_models().await?;
        self.cache().put("openai", models.clone()).await;
        Ok(models)
    }

    async fn generate_embedding(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        let body = OpenAIEmbeddingRequest {
            model: model.to_string(),
            input: text.to_string(),
            encoding_format: Some("float".to_string()),
        };

        let url = format!("{}/embeddings", self.base_url);
        let response = self.send_api_request(&url, &body).await?;

        let embedding_response: OpenAIEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        let embedding = embedding_response
            .data
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::RequestFailed("No embedding in response".to_string()))?
            .embedding;

        Ok(embedding)
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        use futures::stream::StreamExt;

        Self::validate_chat_model(&request.model)?;

        let openai_request = Self::build_responses_request(&request, true);
        let url = format!("{}/responses", self.base_url);
        let response = self.send_api_request(&url, &openai_request).await?;

        let stream = crate::providers::sse::buffered_text_stream(
            response
                .bytes_stream()
                .map(|result| result.map_err(|e| ProviderError::NetworkError(e.to_string()))),
        )
        .flat_map(|result| {
            let chunks: Vec<Result<StreamChunk>> = match result {
                Err(e) => vec![Err(e)],
                Ok(text) => parse_sse_events(&text),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

/// Convert a deserialized Responses API response into a `CompletionResponse`.
fn parse_responses_response(
    openai_response: OpenAIResponsesResponse,
) -> Result<CompletionResponse> {
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    if let Some(text) = openai_response.output_text {
        content = text;
    }

    for output_item in openai_response.output {
        match output_item {
            OpenAIOutputItem::Message(msg) => {
                for content_item in msg.content {
                    match content_item {
                        OpenAIContentItem::OutputText {
                            text,
                            annotations: _,
                        } => {
                            if !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str(&text);
                        }
                    }
                }
            }
            OpenAIOutputItem::FunctionCall(func_call) => {
                tool_calls.push(ToolCall {
                    id: func_call.call_id,
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: func_call.name,
                        arguments: func_call.arguments,
                    },
                    index: None,
                });
            }
        }
    }

    let stop_reason = match openai_response.status.as_str() {
        "completed" | "stopped" => Some("stop".to_string()),
        "length_exceeded" => Some("length".to_string()),
        _ => Some(openai_response.status.clone()),
    };

    Ok(CompletionResponse {
        message: Message {
            role: Role::Assistant,
            content,
            content_parts: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
            name: None,
        },
        model: openai_response.model,
        usage: Some(Usage {
            prompt_tokens: openai_response.usage.input_tokens,
            completion_tokens: openai_response.usage.output_tokens,
            total_tokens: openai_response.usage.total_tokens,
        }),
        stop_reason,
    })
}

/// Parse SSE events from a text buffer into StreamChunks.
fn parse_sse_events(text: &str) -> Vec<Result<StreamChunk>> {
    use crate::providers::sse::parse_sse_data_lines;

    parse_sse_data_lines(text)
        .into_iter()
        .filter_map(parse_openai_chunk)
        .collect()
}

/// Convert a single OpenAI SSE data payload into a `StreamChunk`.
fn parse_openai_chunk(data: &str) -> Option<Result<StreamChunk>> {
    let event: OpenAIStreamEvent = serde_json::from_str(data).ok()?;

    match event.event_type.as_str() {
        "response.output_text.delta" => {
            let delta = event.delta?;
            Some(Ok(StreamChunk {
                delta,
                tool_calls: None,
                usage: None,
                stop_reason: None,
                model: None,
            }))
        }
        "response.function_call_arguments.delta" => {
            let delta = event.delta?;
            let call_id = event
                .item
                .as_ref()
                .and_then(|i| i.get("call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = event
                .item
                .as_ref()
                .and_then(|i| i.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(Ok(StreamChunk {
                delta: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: call_id,
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name,
                        arguments: delta,
                    },
                    index: event.output_index,
                }]),
                usage: None,
                stop_reason: None,
                model: None,
            }))
        }
        "response.completed" => {
            let resp = event.response?;
            let stop_reason = match resp.status.as_str() {
                "completed" | "stopped" => Some("stop".to_string()),
                "length_exceeded" => Some("length".to_string()),
                other => Some(other.to_string()),
            };
            Some(Ok(StreamChunk {
                delta: String::new(),
                tool_calls: None,
                usage: Some(Usage {
                    prompt_tokens: resp.usage.input_tokens,
                    completion_tokens: resp.usage.output_tokens,
                    total_tokens: resp.usage.total_tokens,
                }),
                stop_reason,
                model: Some(resp.model),
            }))
        }
        _ => None,
    }
}
