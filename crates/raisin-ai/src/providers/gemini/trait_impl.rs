//! AIProviderTrait implementation for GeminiProvider.

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
use super::GeminiProvider;

#[async_trait]
impl AIProviderTrait for GeminiProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Filter out system messages (handled separately)
        let non_system_messages: Vec<_> = request
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .cloned()
            .collect();

        let contents = Self::convert_messages_to_contents(&non_system_messages);
        let system_instruction =
            Self::extract_system_prompt(&request.messages, request.system.as_ref());
        let tools = Self::convert_tools(&request.tools);

        let generation_config = GeminiGenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_tokens,
            top_p: None,
            top_k: None,
        };

        let gemini_request = GeminiGenerateRequest {
            contents,
            system_instruction,
            generation_config: Some(generation_config),
            tools,
        };

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, request.model, self.api_key.expose()
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();

            // Try to parse as Gemini error
            if let Ok(error) = serde_json::from_str::<GeminiError>(&error_text) {
                return Err(match error.error.status.as_deref() {
                    Some("INVALID_ARGUMENT") => ProviderError::RequestFailed(error.error.message),
                    Some("UNAUTHENTICATED") => ProviderError::InvalidApiKey,
                    Some("RESOURCE_EXHAUSTED") => ProviderError::RateLimitExceeded,
                    Some("NOT_FOUND") => ProviderError::InvalidModel(error.error.message),
                    _ => ProviderError::RequestFailed(error.error.message),
                });
            }

            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let gemini_response: GeminiGenerateResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Extract content from first candidate
        let candidate = gemini_response
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::RequestFailed("No candidates in response".to_string()))?;

        // Parse response content and tool calls
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut call_counter = 0;

        for part in candidate.content.parts {
            match part {
                GeminiPart::Text { text } => {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&text);
                }
                GeminiPart::FunctionCall { function_call } => {
                    call_counter += 1;
                    tool_calls.push(ToolCall {
                        id: format!("call_{}", call_counter),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: function_call.name,
                            arguments: serde_json::to_string(&function_call.args)
                                .unwrap_or_default(),
                        },
                        index: None,
                    });
                }
                GeminiPart::FunctionResponse { .. } => {
                    // Response shouldn't contain function responses
                }
            }
        }

        // Map finish reason
        let stop_reason = candidate.finish_reason.map(|reason| match reason.as_str() {
            "STOP" => "stop".to_string(),
            "MAX_TOKENS" => "length".to_string(),
            "SAFETY" => "content_filter".to_string(),
            "RECITATION" => "content_filter".to_string(),
            other => other.to_lowercase(),
        });

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
            model: request.model,
            usage: gemini_response.usage_metadata.map(|u| Usage {
                prompt_tokens: u.prompt_token_count,
                completion_tokens: u.candidates_token_count,
                total_tokens: u.total_token_count,
            }),
            stop_reason,
        })
    }

    fn provider_name(&self) -> &str {
        "gemini"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "gemini-2.0-flash-exp".to_string(),
            "gemini-1.5-pro".to_string(),
            "gemini-1.5-pro-latest".to_string(),
            "gemini-1.5-flash".to_string(),
            "gemini-1.5-flash-latest".to_string(),
            "gemini-1.5-flash-8b".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        // Check cache first
        if let Some(cached) = self.cache.get("gemini").await {
            return Ok(cached);
        }

        // Fetch from API
        let models = self.fetch_models().await?;

        // Cache the results
        self.cache.put("gemini", models.clone()).await;

        Ok(models)
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        use futures::stream::StreamExt;

        let non_system_messages: Vec<_> = request
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .cloned()
            .collect();

        let contents = Self::convert_messages_to_contents(&non_system_messages);
        let system_instruction =
            Self::extract_system_prompt(&request.messages, request.system.as_ref());
        let tools = Self::convert_tools(&request.tools);

        let generation_config = GeminiGenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_tokens,
            top_p: None,
            top_k: None,
        };

        let gemini_request = GeminiGenerateRequest {
            contents,
            system_instruction,
            generation_config: Some(generation_config),
            tools,
        };

        // Gemini streaming uses streamGenerateContent instead of generateContent
        let url = format!(
            "{}/models/{}:streamGenerateContent?key={}&alt=sse",
            self.base_url, request.model, self.api_key.expose()
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            if let Ok(error) = serde_json::from_str::<GeminiError>(&error_text) {
                return Err(match error.error.status.as_deref() {
                    Some("UNAUTHENTICATED") => ProviderError::InvalidApiKey,
                    Some("RESOURCE_EXHAUSTED") => ProviderError::RateLimitExceeded,
                    Some("NOT_FOUND") => ProviderError::InvalidModel(error.error.message),
                    _ => ProviderError::RequestFailed(error.error.message),
                });
            }
            return Err(ProviderError::RequestFailed(format!(
                "HTTP {}: {}",
                status, error_text
            )));
        }

        let model_name = request.model.clone();
        let stream = crate::providers::sse::buffered_text_stream(
            response
                .bytes_stream()
                .map(|result| result.map_err(|e| ProviderError::NetworkError(e.to_string()))),
        )
        .flat_map(move |result| {
            let model = model_name.clone();
            let chunks: Vec<Result<StreamChunk>> = match result {
                Err(e) => vec![Err(e)],
                Ok(text) => parse_gemini_sse_events(&text, &model),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

/// Parse SSE events from Gemini's streaming response.
///
/// Gemini with `alt=sse` returns standard SSE `data:` lines containing
/// JSON objects matching the `GeminiGenerateResponse` schema.
fn parse_gemini_sse_events(text: &str, model: &str) -> Vec<Result<StreamChunk>> {
    use crate::providers::sse::parse_sse_data_lines;

    parse_sse_data_lines(text)
        .into_iter()
        .filter_map(|data| parse_gemini_chunk(data, model))
        .collect()
}

/// Convert a single Gemini SSE data payload into a `StreamChunk`.
fn parse_gemini_chunk(data: &str, model: &str) -> Option<Result<StreamChunk>> {
    let response: GeminiGenerateResponse = serde_json::from_str(data).ok()?;

    let candidate = response.candidates.into_iter().next()?;

    let mut text_delta = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut call_counter = 0;

    for part in candidate.content.parts {
        match part {
            GeminiPart::Text { text } => text_delta.push_str(&text),
            GeminiPart::FunctionCall { function_call } => {
                call_counter += 1;
                tool_calls.push(ToolCall {
                    id: format!("call_{}", call_counter),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: function_call.name,
                        arguments: serde_json::to_string(&function_call.args)
                            .unwrap_or_default(),
                    },
                    index: None,
                });
            }
            GeminiPart::FunctionResponse { .. } => {}
        }
    }

    let stop_reason = candidate.finish_reason.map(|reason| match reason.as_str() {
        "STOP" => "stop".to_string(),
        "MAX_TOKENS" => "length".to_string(),
        "SAFETY" | "RECITATION" => "content_filter".to_string(),
        other => other.to_lowercase(),
    });

    let usage = response.usage_metadata.map(|u| Usage {
        prompt_tokens: u.prompt_token_count,
        completion_tokens: u.candidates_token_count,
        total_tokens: u.total_token_count,
    });

    let is_final = usage.is_some() || stop_reason.is_some();

    Some(Ok(StreamChunk {
        delta: text_delta,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        usage,
        stop_reason,
        model: if is_final {
            Some(model.to_string())
        } else {
            None
        },
    }))
}
