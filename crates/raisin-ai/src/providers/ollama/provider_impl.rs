//! AIProviderTrait implementation for Ollama.
//!
//! Handles chat completions, model listing, and embedding generation
//! via the Ollama HTTP API.

use super::api_types::*;
use super::{OllamaProvider, TOOL_CAPABLE_MODELS};
use crate::model_cache::ModelInfo;
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{
    CompletionRequest, CompletionResponse, FunctionCall, Message, ResponseFormat, Role,
    StreamChunk, ToolCall, ToolDefinition, Usage,
};
use crate::utils::strip_markdown_fences;
use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;

#[async_trait]
impl AIProviderTrait for OllamaProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Validate tool support before proceeding
        if request.tools.is_some() && !self.check_tool_support(&request.model).await {
            return Err(ProviderError::UnsupportedOperation(format!(
                "Model '{}' does not support tool calling. Tool-capable models include: {}",
                request.model,
                TOOL_CAPABLE_MODELS.join(", ")
            )));
        }

        let options = if request.temperature.is_some() || request.max_tokens.is_some() {
            Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
            })
        } else {
            None
        };

        // Convert messages
        let mut ollama_messages: Vec<OllamaMessage> = Vec::new();

        // Add user's system message first
        if let Some(system) = &request.system {
            ollama_messages.push(OllamaMessage {
                role: "system".to_string(),
                content: system.clone(),
                images: None,
                tool_calls: None,
            });
        }

        // Add tool guidance AFTER user's system prompt (more recent = more prominent to model)
        if let Some(ref tools) = request.tools {
            if !tools.is_empty() {
                let tool_guidance = ToolDefinition::generate_tool_guidance(tools);
                ollama_messages.push(OllamaMessage {
                    role: "system".to_string(),
                    content: tool_guidance,
                    images: None,
                    tool_calls: None,
                });
            }
        }

        // Add JSON schema instruction for structured output
        // Ollama's format parameter enables grammar enforcement but the model doesn't see the schema.
        // We need to inject it into the prompt so the model knows what structure to produce.
        if let Some(ResponseFormat::JsonSchema { schema }) = &request.response_format {
            let schema_json = serde_json::to_string_pretty(&schema.schema)
                .unwrap_or_else(|_| schema.schema.to_string());

            let json_instruction = format!(
                "IMPORTANT: You MUST respond with valid JSON matching this exact schema:\n\n```json\n{}\n```\n\nRespond ONLY with the JSON object, no markdown formatting, no explanations.",
                schema_json
            );

            ollama_messages.push(OllamaMessage {
                role: "system".to_string(),
                content: json_instruction,
                images: None,
                tool_calls: None,
            });
        }

        // Add conversation messages
        for msg in &request.messages {
            ollama_messages.push(Self::convert_message(msg));
        }

        let tools = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| OllamaTool {
                    tool_type: tool.tool_type.clone(),
                    function: OllamaFunctionDefinition {
                        name: tool.function.name.clone(),
                        description: tool.function.description.clone(),
                        parameters: tool.function.parameters.clone(),
                    },
                })
                .collect()
        });

        // Convert response_format to Ollama's format parameter
        // Save this flag before moving format into the request struct
        let needs_response_cleanup = request.response_format.is_some()
            && !matches!(request.response_format, Some(ResponseFormat::Text));
        let format = request.response_format.as_ref().and_then(|rf| match rf {
            ResponseFormat::Text => None, // Default, no format needed
            ResponseFormat::JsonObject => Some(serde_json::json!("json")),
            ResponseFormat::JsonSchema { schema } => {
                // Ollama supports schema in format field (newer versions)
                // Use the schema directly - Ollama expects the JSON schema object
                Some(schema.schema.clone())
            }
        });

        let ollama_request = OllamaChatRequest {
            model: request.model.clone(),
            messages: ollama_messages,
            tools,
            stream: Some(false),
            options,
            format,
        };

        let request = self
            .client
            .post(format!("{}/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&ollama_request);
        let request = self.add_auth_header(request);
        let response = request
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            if let Ok(error) = serde_json::from_str::<OllamaError>(&error_text) {
                return Err(ProviderError::RequestFailed(error.error));
            }

            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let ollama_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Clean up response content when response_format is set
        // (Tool calls don't need cleanup - they're already structured)
        let content = if needs_response_cleanup {
            strip_markdown_fences(&ollama_response.message.content)
        } else {
            ollama_response.message.content.clone()
        };

        let tool_calls = ollama_response.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|call| ToolCall {
                    id: format!("call_{}", uuid::Uuid::new_v4()),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: call.function.name,
                        arguments: serde_json::to_string(&call.function.arguments)
                            .unwrap_or_default(),
                    },
                    index: None,
                })
                .collect()
        });

        let role = match ollama_response.message.role.as_str() {
            "assistant" => Role::Assistant,
            "user" => Role::User,
            "system" => Role::System,
            "tool" => Role::Tool,
            _ => Role::Assistant,
        };

        Ok(CompletionResponse {
            message: Message {
                role,
                content,
                content_parts: None,
                tool_calls,
                tool_call_id: None,
                name: None,
            },
            model: ollama_response.model,
            usage: Some(Usage {
                prompt_tokens: ollama_response.prompt_eval_count.unwrap_or(0),
                completion_tokens: ollama_response.eval_count.unwrap_or(0),
                total_tokens: ollama_response.prompt_eval_count.unwrap_or(0)
                    + ollama_response.eval_count.unwrap_or(0),
            }),
            stop_reason: if ollama_response.done {
                Some("stop".to_string())
            } else {
                None
            },
        })
    }

    fn provider_name(&self) -> &str {
        "ollama"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        // Ollama supports any installed model - these are just common examples
        vec![
            "llama3.3".to_string(),
            "llama3.2".to_string(),
            "mistral".to_string(),
            "qwen2.5".to_string(),
            "phi3".to_string(),
            "gemma2".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        // Check cache first
        if let Some(cached) = self.cache.get("ollama").await {
            return Ok(cached);
        }

        // Fetch from API
        let models = self.fetch_models().await?;

        // Cache the results with shorter TTL since local models can change frequently
        self.cache.put("ollama", models.clone()).await;

        Ok(models)
    }

    async fn generate_embedding(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        let request = OllamaEmbeddingRequest {
            model: model.to_string(),
            input: text.to_string(),
        };

        let http_request = self
            .client
            .post(format!("{}/embed", self.base_url))
            .header("Content-Type", "application/json")
            .json(&request);
        let http_request = self.add_auth_header(http_request);
        let response = http_request.send().await.map_err(|e| {
            ProviderError::ProviderNotAvailable(format!(
                "Failed to connect to Ollama: {}. Is Ollama running?",
                e
            ))
        })?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            if let Ok(error) = serde_json::from_str::<OllamaError>(&error_text) {
                return Err(ProviderError::RequestFailed(error.error));
            }

            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let embedding_response: OllamaEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Extract the first embedding from the response
        let embedding = embedding_response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::RequestFailed("No embedding in response".to_string()))?;

        Ok(embedding)
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        use futures::stream::StreamExt;

        let options = if request.temperature.is_some() || request.max_tokens.is_some() {
            Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
            })
        } else {
            None
        };

        let mut ollama_messages: Vec<OllamaMessage> = Vec::new();
        if let Some(system) = &request.system {
            ollama_messages.push(OllamaMessage {
                role: "system".to_string(),
                content: system.clone(),
                images: None,
                tool_calls: None,
            });
        }
        for msg in &request.messages {
            ollama_messages.push(Self::convert_message(msg));
        }

        let format = request.response_format.as_ref().and_then(|rf| match rf {
            ResponseFormat::Text => None,
            ResponseFormat::JsonObject => Some(serde_json::json!("json")),
            ResponseFormat::JsonSchema { schema } => Some(schema.schema.clone()),
        });

        let ollama_request = OllamaChatRequest {
            model: request.model.clone(),
            messages: ollama_messages,
            tools: None, // Tool calls are not well-supported in streaming mode
            stream: Some(true),
            options,
            format,
        };

        let http_request = self
            .client
            .post(format!("{}/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&ollama_request);
        let http_request = self.add_auth_header(http_request);
        let response = http_request.send().await.map_err(|e| {
            ProviderError::ProviderNotAvailable(format!(
                "Failed to connect to Ollama: {}. Is Ollama running?",
                e
            ))
        })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            if let Ok(error) = serde_json::from_str::<OllamaError>(&error_text) {
                return Err(ProviderError::RequestFailed(error.error));
            }
            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        // Ollama streaming returns NDJSON: each line is an OllamaChatResponse
        let stream = crate::providers::sse::buffered_text_stream(
            response
                .bytes_stream()
                .map(|result| result.map_err(|e| ProviderError::NetworkError(e.to_string()))),
        )
        .flat_map(|result| {
            let chunks: Vec<Result<StreamChunk>> = match result {
                Err(e) => vec![Err(e)],
                Ok(text) => parse_ollama_ndjson(&text),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

/// Parse NDJSON lines from Ollama's streaming response into `StreamChunk`s.
fn parse_ollama_ndjson(text: &str) -> Vec<Result<StreamChunk>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let response: OllamaChatResponse = serde_json::from_str(line).ok()?;
            let delta = response.message.content.clone();

            if response.done {
                Some(Ok(StreamChunk {
                    delta,
                    tool_calls: None,
                    usage: Some(Usage {
                        prompt_tokens: response.prompt_eval_count.unwrap_or(0),
                        completion_tokens: response.eval_count.unwrap_or(0),
                        total_tokens: response.prompt_eval_count.unwrap_or(0)
                            + response.eval_count.unwrap_or(0),
                    }),
                    stop_reason: Some("stop".to_string()),
                    model: Some(response.model),
                }))
            } else if !delta.is_empty() {
                Some(Ok(StreamChunk {
                    delta,
                    tool_calls: None,
                    usage: None,
                    stop_reason: None,
                    model: None,
                }))
            } else {
                None
            }
        })
        .collect()
}
