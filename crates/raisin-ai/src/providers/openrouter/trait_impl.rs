//! AIProviderTrait implementation for OpenRouterProvider.

use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;

use crate::model_cache::ModelInfo;
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{
    CompletionRequest, CompletionResponse, FunctionCall, Message, ResponseFormat, Role, StreamChunk,
    ToolCall, Usage,
};
use crate::utils::strip_markdown_fences;

use super::types::*;
use super::OpenRouterProvider;

#[async_trait]
impl AIProviderTrait for OpenRouterProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Convert messages to OpenAI format, including system messages
        let mut messages = Vec::new();

        // Add system message if present
        if let Some(system) = &request.system {
            messages.push(OpenAIChatMessage {
                role: "system".to_string(),
                content: Some(system.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add other messages
        messages.extend(Self::convert_messages(&request.messages));

        // Convert tools to OpenAI format
        let converted_tools = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| serde_json::to_value(tool).unwrap_or_default())
                .collect()
        });

        // Convert response_format to OpenRouter format
        // OpenRouter supports both json_object and json_schema (model dependent)
        let response_format = request.response_format.as_ref().and_then(|rf| match rf {
            ResponseFormat::Text => None,
            ResponseFormat::JsonObject => Some(OpenRouterResponseFormat::JsonObject),
            ResponseFormat::JsonSchema { schema } => Some(OpenRouterResponseFormat::JsonSchema {
                schema: OpenRouterJsonSchema {
                    name: schema.name.clone(),
                    schema: schema.schema.clone(),
                    strict: if schema.strict { Some(true) } else { None },
                },
            }),
        });

        let openai_request = OpenAIChatRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: converted_tools,
            stream: if request.stream { Some(true) } else { None },
            response_format,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key.expose()))
            .header("HTTP-Referer", &self.http_referer)
            .header("X-Title", &self.app_name)
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            // Try to parse as OpenAI error
            if let Ok(error) = serde_json::from_str::<OpenAIError>(&error_text) {
                return Err(match error.error.error_type.as_deref() {
                    Some("invalid_request_error") => {
                        ProviderError::RequestFailed(error.error.message)
                    }
                    Some("authentication_error") => ProviderError::InvalidApiKey,
                    Some("rate_limit_error") => ProviderError::RateLimitExceeded,
                    _ => ProviderError::RequestFailed(error.error.message),
                });
            }

            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let openai_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Extract the first choice
        let choice =
            openai_response.choices.into_iter().next().ok_or_else(|| {
                ProviderError::RequestFailed("No choices in response".to_string())
            })?;

        // Parse message content and tool calls
        // Clean up response content when response_format is set
        // (Tool calls don't need cleanup - they're already structured)
        let raw_content = choice.message.content.unwrap_or_default();
        let content = if request.response_format.is_some() {
            strip_markdown_fences(&raw_content)
        } else {
            raw_content
        };

        let tool_calls = choice.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|call| ToolCall {
                    id: call.id,
                    call_type: call.call_type,
                    function: FunctionCall {
                        name: call.function.name,
                        arguments: call.function.arguments,
                    },
                    index: None,
                })
                .collect()
        });

        Ok(CompletionResponse {
            message: Message {
                role: Role::Assistant,
                content,
                content_parts: None,
                tool_calls,
                tool_call_id: None,
                name: None,
            },
            model: openai_response.model,
            usage: Some(Usage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                completion_tokens: openai_response.usage.completion_tokens,
                total_tokens: openai_response.usage.total_tokens,
            }),
            stop_reason: choice.finish_reason,
        })
    }

    fn provider_name(&self) -> &str {
        "openrouter"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "openai/gpt-4o".to_string(),
            "openai/gpt-4-turbo".to_string(),
            "anthropic/claude-3-sonnet".to_string(),
            "anthropic/claude-3-opus".to_string(),
            "meta-llama/llama-3.3-70b-instruct".to_string(),
            "google/gemini-pro".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        // Check cache first
        if let Some(cached) = self.cache.get("openrouter").await {
            return Ok(cached);
        }

        // Fetch from API
        let models = self.fetch_models().await?;

        // Cache the results
        self.cache.put("openrouter", models.clone()).await;

        Ok(models)
    }

    async fn generate_embedding(&self, _text: &str, _model: &str) -> Result<Vec<f32>> {
        Err(ProviderError::UnsupportedOperation(
            "OpenRouter does not support embeddings through this API".to_string(),
        ))
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        use futures::stream::StreamExt;

        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(OpenAIChatMessage {
                role: "system".to_string(),
                content: Some(system.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        messages.extend(Self::convert_messages(&request.messages));

        let converted_tools = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| serde_json::to_value(tool).unwrap_or_default())
                .collect()
        });

        let response_format = request.response_format.as_ref().and_then(|rf| match rf {
            ResponseFormat::Text => None,
            ResponseFormat::JsonObject => Some(OpenRouterResponseFormat::JsonObject),
            ResponseFormat::JsonSchema { schema } => Some(OpenRouterResponseFormat::JsonSchema {
                schema: OpenRouterJsonSchema {
                    name: schema.name.clone(),
                    schema: schema.schema.clone(),
                    strict: if schema.strict { Some(true) } else { None },
                },
            }),
        });

        let openai_request = OpenAIChatRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: converted_tools,
            stream: Some(true),
            response_format,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key.expose()))
            .header("HTTP-Referer", &self.http_referer)
            .header("X-Title", &self.app_name)
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            if let Ok(error) = serde_json::from_str::<OpenAIError>(&error_text) {
                return Err(match error.error.error_type.as_deref() {
                    Some("authentication_error") => ProviderError::InvalidApiKey,
                    Some("rate_limit_error") => ProviderError::RateLimitExceeded,
                    _ => ProviderError::RequestFailed(error.error.message),
                });
            }
            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let stream = crate::providers::sse::buffered_text_stream(
            response
                .bytes_stream()
                .map(|result| result.map_err(|e| ProviderError::NetworkError(e.to_string()))),
        )
        .flat_map(|result| {
            let chunks: Vec<Result<StreamChunk>> = match result {
                Err(e) => vec![Err(e)],
                Ok(text) => parse_openrouter_sse_events(&text),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

/// Parse SSE events from OpenRouter's OpenAI-compatible streaming response.
fn parse_openrouter_sse_events(text: &str) -> Vec<Result<StreamChunk>> {
    use crate::providers::sse::parse_sse_data_lines;

    parse_sse_data_lines(text)
        .into_iter()
        .flat_map(parse_openrouter_chunk)
        .collect()
}

/// Convert a single OpenRouter SSE data payload into zero or more `StreamChunk`s.
fn parse_openrouter_chunk(data: &str) -> Vec<Result<StreamChunk>> {
    let mut chunks = Vec::new();

    let chunk: OpenRouterStreamChunk = match serde_json::from_str(data) {
        Ok(c) => c,
        Err(_) => return chunks,
    };

    for choice in &chunk.choices {
        if let Some(content) = &choice.delta.content {
            if !content.is_empty() {
                chunks.push(Ok(StreamChunk {
                    delta: content.clone(),
                    tool_calls: None,
                    usage: None,
                    stop_reason: None,
                    model: None,
                }));
            }
        }

        if let Some(tool_calls) = &choice.delta.tool_calls {
            for tc in tool_calls {
                let id = tc.id.clone().unwrap_or_default();
                let (name, arguments) = match &tc.function {
                    Some(f) => (
                        f.name.clone().unwrap_or_default(),
                        f.arguments.clone().unwrap_or_default(),
                    ),
                    None => (String::new(), String::new()),
                };
                chunks.push(Ok(StreamChunk {
                    delta: String::new(),
                    tool_calls: Some(vec![ToolCall {
                        id,
                        call_type: "function".to_string(),
                        function: FunctionCall { name, arguments },
                        index: Some(tc.index),
                    }]),
                    usage: None,
                    stop_reason: None,
                    model: None,
                }));
            }
        }

        if let Some(reason) = &choice.finish_reason {
            chunks.push(Ok(StreamChunk {
                delta: String::new(),
                tool_calls: None,
                usage: chunk.usage.as_ref().map(|u| Usage {
                    prompt_tokens: u.prompt_tokens,
                    completion_tokens: u.completion_tokens,
                    total_tokens: u.total_tokens,
                }),
                stop_reason: Some(reason.clone()),
                model: chunk.model.clone(),
            }));
        }
    }

    chunks
}
