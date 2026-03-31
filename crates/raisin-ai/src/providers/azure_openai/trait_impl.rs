//! AIProviderTrait implementation for AzureOpenAIProvider.

use async_trait::async_trait;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::model_cache::ModelInfo;
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{
    CompletionRequest, CompletionResponse, FunctionCall, Message, Role, StreamChunk, ToolCall,
    Usage,
};

use super::types::*;
use super::AzureOpenAIProvider;

#[async_trait]
impl AIProviderTrait for AzureOpenAIProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Build messages including system prompt if provided
        let mut messages: Vec<AzureChatMessage> = Vec::new();

        // Add system message if provided in request
        if let Some(ref system) = request.system {
            messages.push(AzureChatMessage::System {
                content: system.clone(),
            });
        }

        // Add conversation messages
        for msg in &request.messages {
            messages.push(Self::convert_message(msg));
        }

        let tools = Self::convert_tools(&request.tools);

        let azure_request = AzureChatRequest {
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools,
            stream: None,
            stream_options: None,
        };

        // Build URL with deployment name (model) and API version
        let url = format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, request.model, self.api_version
        );

        let response = self
            .client
            .post(&url)
            .header("api-key", self.api_key.expose())
            .header("Content-Type", "application/json")
            .json(&azure_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            // Try to parse as Azure error
            if let Ok(error) = serde_json::from_str::<AzureError>(&error_text) {
                return Err(match error.error.code.as_deref() {
                    Some("invalid_request_error") => {
                        ProviderError::RequestFailed(error.error.message)
                    }
                    Some("401") | Some("Unauthorized") => ProviderError::InvalidApiKey,
                    Some("429") | Some("RateLimitExceeded") => ProviderError::RateLimitExceeded,
                    Some("DeploymentNotFound") => ProviderError::InvalidModel(error.error.message),
                    _ => ProviderError::RequestFailed(error.error.message),
                });
            }

            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let azure_response: AzureChatResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Extract first choice
        let choice =
            azure_response.choices.into_iter().next().ok_or_else(|| {
                ProviderError::RequestFailed("No choices in response".to_string())
            })?;

        // Convert tool calls
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                },
                index: None,
            })
            .collect();

        // Map finish reason
        let stop_reason = choice.finish_reason.map(|reason| match reason.as_str() {
            "stop" => "stop".to_string(),
            "length" => "length".to_string(),
            "tool_calls" => "tool_use".to_string(),
            "content_filter" => "content_filter".to_string(),
            other => other.to_string(),
        });

        Ok(CompletionResponse {
            message: Message {
                role: Role::Assistant,
                content: choice.message.content.unwrap_or_default(),
                content_parts: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
                name: None,
            },
            model: azure_response.model,
            usage: Some(Usage {
                prompt_tokens: azure_response.usage.prompt_tokens,
                completion_tokens: azure_response.usage.completion_tokens,
                total_tokens: azure_response.usage.total_tokens,
            }),
            stop_reason,
        })
    }

    fn provider_name(&self) -> &str {
        "azure_openai"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        // Azure OpenAI uses deployment names, which are custom per resource
        // Return common deployment patterns
        vec![
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "gpt-4".to_string(),
            "gpt-4-32k".to_string(),
            "gpt-35-turbo".to_string(),
            "gpt-35-turbo-16k".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        // Check cache first
        if let Some(cached) = self.cache.get("azure_openai").await {
            return Ok(cached);
        }

        // Azure doesn't have a models endpoint like OpenAI
        // Return static list with capabilities
        let models: Vec<ModelInfo> = self
            .available_models()
            .into_iter()
            .map(|name| self.build_model_info(&name))
            .collect();

        // Cache the results
        self.cache.put("azure_openai", models.clone()).await;

        Ok(models)
    }

    async fn generate_embedding(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        #[derive(Serialize)]
        struct EmbeddingRequest {
            input: String,
        }

        #[derive(Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
        }

        #[derive(Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }

        let url = format!(
            "{}/openai/deployments/{}/embeddings?api-version={}",
            self.endpoint, model, self.api_version
        );

        let response = self
            .client
            .post(&url)
            .header("api-key", self.api_key.expose())
            .header("Content-Type", "application/json")
            .json(&EmbeddingRequest {
                input: text.to_string(),
            })
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let embedding_response: EmbeddingResponse = response
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

        let mut messages: Vec<AzureChatMessage> = Vec::new();
        if let Some(ref system) = request.system {
            messages.push(AzureChatMessage::System {
                content: system.clone(),
            });
        }
        for msg in &request.messages {
            messages.push(Self::convert_message(msg));
        }

        let tools = Self::convert_tools(&request.tools);

        let azure_request = AzureChatRequest {
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools,
            stream: Some(true),
            stream_options: Some(AzureStreamOptions {
                include_usage: true,
            }),
        };

        let url = format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, request.model, self.api_version
        );

        let response = self
            .client
            .post(&url)
            .header("api-key", self.api_key.expose())
            .header("Content-Type", "application/json")
            .json(&azure_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            if let Ok(error) = serde_json::from_str::<AzureError>(&error_text) {
                return Err(match error.error.code.as_deref() {
                    Some("401") | Some("Unauthorized") => ProviderError::InvalidApiKey,
                    Some("429") | Some("RateLimitExceeded") => ProviderError::RateLimitExceeded,
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
                Ok(text) => parse_azure_sse_events(&text),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

/// Parse SSE events from Azure OpenAI's streaming response.
fn parse_azure_sse_events(text: &str) -> Vec<Result<StreamChunk>> {
    use crate::providers::sse::parse_sse_data_lines;

    parse_sse_data_lines(text)
        .into_iter()
        .flat_map(parse_azure_chunk)
        .collect()
}

/// Convert a single Azure OpenAI SSE data payload into zero or more `StreamChunk`s.
fn parse_azure_chunk(data: &str) -> Vec<Result<StreamChunk>> {
    let mut chunks = Vec::new();

    let chunk: AzureStreamChunk = match serde_json::from_str(data) {
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
