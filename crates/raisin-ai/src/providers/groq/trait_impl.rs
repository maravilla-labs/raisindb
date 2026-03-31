//! AIProviderTrait implementation for GroqProvider.

use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;

use crate::model_cache::ModelInfo;
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{
    CompletionRequest, CompletionResponse, FunctionCall, Message, Role, StreamChunk, ToolCall,
    Usage,
};
use crate::utils::strip_markdown_fences;

use super::types::*;
use super::GroqProvider;

/// Parse Llama-style tool calls from a failed_generation error message.
///
/// Delegates to the shared [`crate::tool_call_extraction`] module which handles
/// `<function=name>{args}</function>` patterns (including hyphenated names).
fn parse_llama_tool_calls(error_msg: &str) -> Option<Vec<ToolCall>> {
    crate::tool_call_extraction::extract_tool_calls_from_content(error_msg)
}

#[async_trait]
impl AIProviderTrait for GroqProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        Self::validate_chat_model(&request.model)?;

        let groq_request = Self::build_chat_request(&request, false);
        let response = match self.send_api_request(&groq_request).await {
            Ok(resp) => resp,
            Err(ProviderError::RequestFailed(ref msg)) if msg.contains("<function=") => {
                if let Some(tool_calls) = parse_llama_tool_calls(msg) {
                    return Ok(CompletionResponse {
                        message: Message::assistant("").with_tool_calls(tool_calls),
                        model: request.model.clone(),
                        usage: None,
                        stop_reason: Some("tool_calls".to_string()),
                    });
                }
                return Err(ProviderError::RequestFailed(msg.clone()));
            }
            Err(e) => return Err(e),
        };

        let groq_response: GroqChatResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        let choice =
            groq_response.choices.into_iter().next().ok_or_else(|| {
                ProviderError::RequestFailed("No choices in response".to_string())
            })?;

        let tool_calls = choice.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|tc| ToolCall {
                    id: tc.id,
                    call_type: tc.call_type,
                    function: FunctionCall {
                        name: tc.function.name,
                        arguments: tc.function.arguments,
                    },
                    index: None,
                })
                .collect()
        });

        let stop_reason = choice.finish_reason.or(Some("stop".to_string()));

        let raw_content = choice.message.content.unwrap_or_default();
        let content = if request.response_format.is_some() {
            strip_markdown_fences(&raw_content)
        } else {
            raw_content
        };

        let mut response = CompletionResponse {
            message: Message {
                role: Role::Assistant,
                content,
                content_parts: None,
                tool_calls,
                tool_call_id: None,
                name: None,
            },
            model: groq_response.model,
            usage: Some(Usage {
                prompt_tokens: groq_response.usage.prompt_tokens,
                completion_tokens: groq_response.usage.completion_tokens,
                total_tokens: groq_response.usage.total_tokens,
            }),
            stop_reason,
        };

        Self::extract_structured_output(&mut response, request.response_format.as_ref());

        Ok(response)
    }

    fn provider_name(&self) -> &str {
        "groq"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "llama-3.3-70b-versatile".to_string(),
            "llama-3.1-8b-instant".to_string(),
            "llama-3.1-70b-versatile".to_string(),
            "mixtral-8x7b-32768".to_string(),
            "gemma2-9b-it".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        if let Some(cached) = self.cache.get("groq").await {
            return Ok(cached);
        }

        let models = self.fetch_models().await?;
        self.cache.put("groq", models.clone()).await;

        Ok(models)
    }

    async fn generate_embedding(&self, _text: &str, _model: &str) -> Result<Vec<f32>> {
        Err(ProviderError::UnsupportedOperation(
            "Groq does not support embeddings. Use OpenAI or other providers for embeddings."
                .to_string(),
        ))
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        use futures::stream::StreamExt;

        Self::validate_chat_model(&request.model)?;

        let groq_request = Self::build_chat_request(&request, true);
        let response = match self.send_api_request(&groq_request).await {
            Ok(resp) => resp,
            Err(ProviderError::RequestFailed(ref msg)) if msg.contains("<function=") => {
                if let Some(tool_calls) = parse_llama_tool_calls(msg) {
                    // Emit tool calls as stream chunks followed by a stop chunk
                    let chunks: Vec<Result<StreamChunk>> = tool_calls
                        .into_iter()
                        .enumerate()
                        .map(|(i, tc)| {
                            Ok(StreamChunk {
                                delta: String::new(),
                                tool_calls: Some(vec![ToolCall {
                                    index: Some(i),
                                    ..tc
                                }]),
                                usage: None,
                                stop_reason: None,
                                model: None,
                            })
                        })
                        .chain(std::iter::once(Ok(StreamChunk {
                            delta: String::new(),
                            tool_calls: None,
                            usage: None,
                            stop_reason: Some("tool_calls".to_string()),
                            model: Some(request.model.clone()),
                        })))
                        .collect();
                    return Ok(Box::pin(futures::stream::iter(chunks)));
                }
                return Err(ProviderError::RequestFailed(msg.clone()));
            }
            Err(e) => return Err(e),
        };

        let stream = crate::providers::sse::buffered_text_stream(
            response
                .bytes_stream()
                .map(|result| result.map_err(|e| ProviderError::NetworkError(e.to_string()))),
        )
        .flat_map(|result| {
            let chunks: Vec<Result<StreamChunk>> = match result {
                Err(e) => vec![Err(e)],
                Ok(text) => parse_groq_sse_events(&text),
            };
            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }
}

/// Parse SSE events from Groq's OpenAI-compatible streaming response.
pub(super) fn parse_groq_sse_events(text: &str) -> Vec<Result<StreamChunk>> {
    use crate::providers::sse::parse_sse_data_lines;

    parse_sse_data_lines(text)
        .into_iter()
        .flat_map(parse_groq_chunk)
        .collect()
}

/// Convert a single Groq SSE data payload into zero or more `StreamChunk`s.
fn parse_groq_chunk(data: &str) -> Vec<Result<StreamChunk>> {
    let mut chunks = Vec::new();

    let chunk: GroqStreamChunk = match serde_json::from_str(data) {
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
