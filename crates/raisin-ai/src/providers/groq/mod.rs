//! Groq API provider implementation.
//!
//! Uses Groq's OpenAI-compatible API for chat completions.
//! Groq specializes in fast inference for open-source models.
//!
//! The set of available models is fetched live from Groq's `/models` endpoint
//! ([`GroqProvider::fetch_models`]) rather than hardcoded — Groq adds and
//! decommissions models frequently, so any static allowlist drifts out of sync.
//! Every model Groq returns is listed; Groq's `/models` does not tag
//! capabilities, so per-model tool-call support is inferred by name via
//! [`groq_model_supports_tools`] (per Groq's docs all chat/LLM models support
//! tool use; speech-to-text, text-to-speech, and moderation guards do not).

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

/// Returns `true` if a Groq model supports tool/function calling.
///
/// Groq's `/models` endpoint does not report capabilities, but per Groq's docs
/// all of its chat/LLM models support tool use. The only models that don't are
/// the non-chat ones it returns alongside them: speech-to-text (Whisper),
/// text-to-speech (PlayAI/Orpheus TTS), and moderation/guard classifiers. We
/// detect those by stable name markers — the categories outlive individual
/// model versions, so this is far more durable than an allowlist.
pub fn groq_model_supports_tools(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    const NON_TOOL_MARKERS: &[&str] = &[
        "whisper", // speech-to-text
        "tts",     // text-to-speech
        "orpheus", // text-to-speech (canopylabs)
        "playai",  // text-to-speech
        "guard",   // moderation/classifier (llama-guard, prompt-guard, safeguard)
    ];
    !NON_TOOL_MARKERS.iter().any(|marker| id.contains(marker))
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

        // Convert every Groq model to our ModelInfo format. We list all of them
        // (incl. whisper/TTS/guard) and instead annotate tool-call support per
        // model. Sort by id so the list is stable across refreshes — Groq
        // returns models in a non-deterministic order otherwise.
        let mut models: Vec<ModelInfo> = models_response
            .data
            .into_iter()
            .map(|model| self.convert_groq_model(model))
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(models)
    }

    /// Converts a Groq model to our ModelInfo format.
    ///
    /// Tool-call support is inferred per model via [`groq_model_supports_tools`];
    /// every Groq model is treated as chat-capable so it stays listed.
    ///
    /// This client also serves `AIProvider::Custom`, i.e. any OpenAI-shaped
    /// gateway. Where such a gateway declares `kind`/`dimensions`, that
    /// declaration OVERRIDES the name-based guesses below via
    /// [`crate::model_classifier::apply_declared_kind`] — the gateway knows
    /// what it serves and this client cannot. Real Groq publishes neither key,
    /// so its models come out exactly as before.
    fn convert_groq_model(&self, model: GroqModel) -> ModelInfo {
        let mut capabilities = ModelCapabilities {
            chat: true,
            embeddings: false, // Groq itself provides no embedding models
            vision: false,     // Groq doesn't support vision yet
            tools: groq_model_supports_tools(&model.id),
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

        let mut metadata = serde_json::json!({
            "owned_by": model.owned_by,
            "created": model.created,
            "active": model.active.unwrap_or(true),
        });

        crate::model_classifier::apply_declared_kind(
            &model.id,
            model.kind.as_deref(),
            model.dimensions,
            &mut capabilities,
            &mut metadata,
        );

        ModelInfo::new(model.id.clone(), model.id)
            .with_capabilities(capabilities)
            .with_context_window(context_window)
            .with_metadata(metadata)
    }

    /// Validates the requested chat model.
    ///
    /// Groq's catalog changes frequently, so we do NOT keep a hardcoded
    /// allowlist (it inevitably rejects valid new models and lists
    /// decommissioned ones). We only guard against an empty model name; Groq's
    /// API is the source of truth and returns a clear error for unknown models.
    fn validate_chat_model(model: &str) -> Result<()> {
        if model.trim().is_empty() {
            return Err(ProviderError::InvalidModel(
                "Model name must not be empty".to_string(),
            ));
        }
        Ok(())
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
    ) -> bool {
        super::structured_output::extract_structured_output(response, response_format)
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
