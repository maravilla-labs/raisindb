//! Groq API provider implementation.
//!
//! Uses Groq's OpenAI-compatible API for chat completions.
//! Groq specializes in fast inference for open-source models.
//!
//! Supported models:
//! - llama-3.3-70b-versatile (Meta's Llama 3.3, 70B parameters)
//! - llama-3.1-8b-instant (Meta's Llama 3.1, 8B parameters, optimized for speed)
//! - mixtral-8x7b-32768 (Mistral's Mixtral MoE, 32K context)
//! - gemma2-9b-it (Google's Gemma 2, 9B parameters)

#[cfg(test)]
mod tests;
mod trait_impl;
pub(crate) mod types;

use super::http_helpers::SecretKey;
use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::provider::{ProviderError, Result};
use crate::types::{CompletionRequest, CompletionResponse, Message, ResponseFormat, Role};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use types::*;

const GROQ_API_BASE: &str = "https://api.groq.com/openai/v1";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

use super::structured_output::STRUCTURED_OUTPUT_TOOL;

/// Groq provider configuration
#[derive(Debug, Clone)]
pub struct GroqProvider {
    api_key: SecretKey,
    client: Client,
    base_url: String,
    cache: Arc<ModelCache>,
}

impl GroqProvider {
    /// Creates a new Groq provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: GROQ_API_BASE.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Creates a new Groq provider with custom base URL
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: base_url.into(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Fetches the list of available models from Groq API
    async fn fetch_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key.expose()))
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::RequestFailed(format!(
                "Failed to fetch models: HTTP {}: {}",
                status, error_text
            )));
        }

        let models_response: GroqModelsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Convert Groq models to our ModelInfo format
        let models = models_response
            .data
            .into_iter()
            .map(|model| self.convert_groq_model(model))
            .collect();

        Ok(models)
    }

    /// Converts a Groq model to our ModelInfo format
    fn convert_groq_model(&self, model: GroqModel) -> ModelInfo {
        // All Groq models support chat and streaming
        let supports_tools = !model.id.contains("whisper"); // Audio models don't support tools

        let capabilities = ModelCapabilities {
            chat: true,
            embeddings: false, // Groq doesn't provide embedding models
            vision: false,     // Groq doesn't support vision yet
            tools: supports_tools,
            streaming: true,
        };

        // Determine context window based on model ID
        let context_window = if model.id.contains("32768") {
            32768
        } else if model.id.contains("llama-3.3")
            || model.id.contains("llama-3.1")
            || model.id.contains("llama-3.2")
        {
            128000 // Llama 3.x has extended context
        } else {
            8192 // Default context window
        };

        ModelInfo::new(model.id.clone(), model.id)
            .with_capabilities(capabilities)
            .with_context_window(context_window)
            .with_metadata(serde_json::json!({
                "owned_by": model.owned_by,
                "created": model.created,
                "active": model.active.unwrap_or(true),
            }))
    }

    /// Validates that the model is supported for chat
    fn validate_chat_model(model: &str) -> Result<()> {
        const SUPPORTED_MODELS: &[&str] = &[
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "llama-3.1-70b-versatile",
            "llama-3.2-1b-preview",
            "llama-3.2-3b-preview",
            "llama-3.2-11b-vision-preview",
            "llama-3.2-90b-vision-preview",
            "mixtral-8x7b-32768",
            "gemma2-9b-it",
            "gemma-7b-it",
        ];

        if SUPPORTED_MODELS.iter().any(|m| model.starts_with(m)) {
            Ok(())
        } else {
            Err(ProviderError::InvalidModel(format!(
                "Unsupported chat model: {}. Supported models: {}",
                model,
                SUPPORTED_MODELS.join(", ")
            )))
        }
    }

    /// Applies structured output settings from `ResponseFormat` to the request.
    ///
    /// - `JsonObject`: sets `response_format` to `json_object`.
    /// - `JsonSchema`: injects a synthetic tool whose parameters match the
    ///   requested schema and forces the model to call it via `tool_choice`.
    ///   The caller must later extract the tool output via `extract_structured_output`.
    fn apply_response_format(
        response_format: Option<&ResponseFormat>,
        groq_response_format: &mut Option<GroqResponseFormat>,
        tools: &mut Option<Vec<GroqToolDefinition>>,
        tool_choice: &mut Option<GroqToolChoice>,
    ) {
        let Some(format) = response_format else {
            return;
        };

        match format {
            ResponseFormat::Text => {}
            ResponseFormat::JsonObject => {
                *groq_response_format = Some(GroqResponseFormat {
                    format_type: "json_object".to_string(),
                });
            }
            ResponseFormat::JsonSchema { schema } => {
                let tool_name = schema
                    .name
                    .as_deref()
                    .unwrap_or(STRUCTURED_OUTPUT_TOOL)
                    .to_string();

                let structured_tool = GroqToolDefinition {
                    tool_type: "function".to_string(),
                    function: GroqFunctionDefinition {
                        name: tool_name.clone(),
                        description: Some(
                            "Respond with structured output matching the schema.".to_string(),
                        ),
                        parameters: Some(schema.schema.clone()),
                    },
                };

                match tools {
                    Some(existing) => existing.push(structured_tool),
                    None => *tools = Some(vec![structured_tool]),
                }

                *tool_choice = Some(GroqToolChoice::Specific(GroqToolChoiceSpecific {
                    choice_type: "function".to_string(),
                    function: GroqToolChoiceFunction { name: tool_name },
                }));
            }
        }
    }

    /// Checks whether the response contains a structured output tool call
    /// injected by `apply_response_format` and, if so, moves its JSON payload
    /// into `message.content` so callers get a uniform response shape.
    fn extract_structured_output(
        response: &mut CompletionResponse,
        response_format: Option<&ResponseFormat>,
    ) {
        super::structured_output::extract_structured_output(response, response_format);
    }

    /// Build a `GroqChatRequest` from a `CompletionRequest`, applying response
    /// format and structured output transforms.
    fn build_chat_request(request: &CompletionRequest, stream: bool) -> GroqChatRequest {
        let messages: Vec<GroqMessage> =
            request.messages.iter().map(Self::convert_message).collect();

        let mut converted_tools = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| GroqToolDefinition {
                    tool_type: tool.tool_type.clone(),
                    function: GroqFunctionDefinition {
                        name: tool.function.name.clone(),
                        description: if tool.function.description.is_empty() {
                            None
                        } else {
                            Some(tool.function.description.clone())
                        },
                        parameters: Some(tool.function.parameters.clone()),
                    },
                })
                .collect()
        });

        let mut response_format = None;
        let mut tool_choice = None;
        Self::apply_response_format(
            request.response_format.as_ref(),
            &mut response_format,
            &mut converted_tools,
            &mut tool_choice,
        );

        GroqChatRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: converted_tools,
            tool_choice,
            response_format,
            stream: if stream { Some(true) } else { None },
        }
    }

    /// Send a request to the Groq API and return the raw `reqwest::Response`.
    /// Handles authentication and maps HTTP errors to `ProviderError`.
    async fn send_api_request(&self, groq_request: &GroqChatRequest) -> Result<reqwest::Response> {
        use super::http_helpers;

        http_helpers::send_json_request(
            &self.client,
            &format!("{}/chat/completions", self.base_url),
            ("Authorization", format!("Bearer {}", self.api_key.expose())),
            groq_request,
            &[],
            |r| Box::pin(http_helpers::handle_openai_style_error(r)),
        )
        .await
    }

    /// Converts our Message type to Groq/OpenAI format
    fn convert_message(msg: &Message) -> GroqMessage {
        match msg.role {
            Role::User => GroqMessage {
                role: "user".to_string(),
                content: Some(msg.content.clone()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            Role::Assistant => GroqMessage {
                role: "assistant".to_string(),
                content: if msg.content.is_empty() {
                    None
                } else {
                    Some(msg.content.clone())
                },
                tool_calls: msg.tool_calls.as_ref().map(|calls| {
                    calls
                        .iter()
                        .map(|tc| GroqToolCall {
                            id: tc.id.clone(),
                            call_type: tc.call_type.clone(),
                            function: GroqFunctionCall {
                                name: tc.function.name.clone(),
                                arguments: tc.function.arguments.clone(),
                            },
                        })
                        .collect()
                }),
                tool_call_id: None,
                name: None,
            },
            Role::Tool => GroqMessage {
                role: "tool".to_string(),
                content: Some(msg.content.clone()),
                tool_calls: None,
                tool_call_id: msg.tool_call_id.clone(),
                name: msg.name.clone(),
            },
            Role::System => GroqMessage {
                role: "system".to_string(),
                content: Some(msg.content.clone()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        }
    }
}
