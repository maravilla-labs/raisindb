//! OpenAI API provider implementation.
//!
//! Uses the OpenAI Responses API (/v1/responses) for chat completions
//! and embeddings using OpenAI's API.
//!
//! Supported models:
//! - Chat: gpt-4.1, gpt-4o, gpt-4-turbo, o1, o1-mini
//! - Embeddings: text-embedding-3-large, text-embedding-3-small

#[cfg(test)]
mod tests;
mod trait_impl;
pub(crate) mod types;

use super::http_helpers::SecretKey;
use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::provider::{ProviderError, Result};
use crate::types::{CompletionRequest, Message, ResponseFormat, Role};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use types::*;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600);

/// OpenAI provider configuration
#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    api_key: SecretKey,
    client: Client,
    base_url: String,
    cache: Arc<ModelCache>,
}

impl OpenAIProvider {
    /// Creates a new OpenAI provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: OPENAI_API_BASE.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Creates a new OpenAI provider with custom base URL (for Azure OpenAI, etc.)
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: base_url.into(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Returns reference to the model cache.
    pub(crate) fn cache(&self) -> &ModelCache {
        &self.cache
    }

    // ── Shared helpers ─────────────────────────────────────────────

    /// Build an `OpenAIResponsesRequest` from a `CompletionRequest`.
    ///
    /// Converts messages, tools, and response format into the Responses API
    /// shape. Set `stream` to `true` for streaming requests.
    fn build_responses_request(
        request: &CompletionRequest,
        stream: bool,
    ) -> OpenAIResponsesRequest {
        let input: Vec<OpenAIInputItem> = request
            .messages
            .iter()
            .flat_map(Self::convert_messages)
            .collect();

        let converted_tools = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| {
                    let responses_tool = OpenAIResponsesToolDefinition {
                        tool_type: tool.tool_type.clone(),
                        name: tool.function.name.clone(),
                        description: if tool.function.description.is_empty() {
                            None
                        } else {
                            Some(tool.function.description.clone())
                        },
                        parameters: Some(tool.function.parameters.clone()),
                    };
                    serde_json::to_value(responses_tool).unwrap_or_default()
                })
                .collect()
        });

        let text = request.response_format.as_ref().and_then(|rf| match rf {
            ResponseFormat::Text => None,
            ResponseFormat::JsonObject => Some(OpenAITextSettings {
                format: OpenAIResponseFormat::JsonObject,
            }),
            ResponseFormat::JsonSchema { schema } => Some(OpenAITextSettings {
                format: OpenAIResponseFormat::JsonSchema {
                    schema: OpenAIJsonSchema {
                        name: schema.name.clone(),
                        schema: schema.schema.clone(),
                        strict: if schema.strict { Some(true) } else { None },
                    },
                },
            }),
        });

        OpenAIResponsesRequest {
            model: request.model.clone(),
            input,
            instructions: request.system.clone(),
            temperature: request.temperature,
            max_output_tokens: request.max_tokens,
            tools: converted_tools,
            text,
            stream: if stream { Some(true) } else { None },
        }
    }

    /// Send a JSON-serializable request body to `url` with Bearer auth.
    ///
    /// Returns the raw `reqwest::Response` on success, or maps the HTTP
    /// error into a `ProviderError` on failure.
    async fn send_api_request<T: serde::Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        use super::http_helpers;

        http_helpers::send_json_request(
            &self.client,
            url,
            ("Authorization", format!("Bearer {}", self.api_key.expose())),
            body,
            &[],
            |r| Box::pin(http_helpers::handle_openai_style_error(r)),
        )
        .await
    }

    // ── Model helpers ──────────────────────────────────────────────

    /// Fetches the list of available models from OpenAI API
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

        let models_response: OpenAIModelsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        let models = models_response
            .data
            .into_iter()
            .filter_map(|model| {
                if model.id.starts_with("gpt-")
                    || model.id.starts_with("o1")
                    || model.id.starts_with("text-embedding")
                {
                    Some(self.convert_openai_model(model))
                } else {
                    None
                }
            })
            .collect();

        Ok(models)
    }

    /// Converts an OpenAI model to our ModelInfo format
    fn convert_openai_model(&self, model: OpenAIModel) -> ModelInfo {
        let is_chat = model.id.starts_with("gpt-") || model.id.starts_with("o1");
        let is_embedding = model.id.starts_with("text-embedding");
        let is_vision = model.id.contains("vision") || model.id.contains("gpt-4");
        let supports_tools =
            is_chat && !model.id.starts_with("o1") && !model.id.contains("gpt-3.5");

        let capabilities = ModelCapabilities {
            chat: is_chat,
            embeddings: is_embedding,
            vision: is_vision,
            tools: supports_tools,
            streaming: is_chat,
        };

        let context_window = if model.id.contains("gpt-4-turbo") || model.id.contains("gpt-4o") {
            Some(128000)
        } else if model.id.contains("gpt-4") {
            Some(8192)
        } else if model.id.contains("gpt-3.5-turbo-16k") {
            Some(16384)
        } else if model.id.contains("gpt-3.5") {
            Some(4096)
        } else if model.id.starts_with("o1") {
            Some(200000)
        } else {
            None
        };

        let mut metadata = serde_json::json!({
            "owned_by": model.owned_by,
            "created": model.created,
        });

        if is_embedding {
            let embedding_length = match model.id.as_str() {
                "text-embedding-3-small" => 1536,
                "text-embedding-3-large" => 3072,
                "text-embedding-ada-002" => 1536,
                _ => 1536,
            };
            metadata["embedding_length"] = serde_json::json!(embedding_length);
        }

        ModelInfo::new(model.id.clone(), model.id)
            .with_capabilities(capabilities)
            .with_context_window(context_window.unwrap_or(4096))
            .with_metadata(metadata)
    }

    /// Validates that the model is supported for chat
    fn validate_chat_model(model: &str) -> Result<()> {
        const SUPPORTED_MODELS: &[&str] = &[
            "gpt-4.1",
            "gpt-4o",
            "gpt-4-turbo",
            "gpt-4",
            "gpt-3.5-turbo",
            "o1",
            "o1-mini",
            "o1-preview",
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

    // ── Message conversion ─────────────────────────────────────────

    /// Converts our Message type to OpenAI Responses API input format
    fn convert_messages(msg: &Message) -> Vec<OpenAIInputItem> {
        match msg.role {
            Role::User => vec![OpenAIInputItem::Message(OpenAIMessage {
                role: "user".to_string(),
                content: msg.content.clone(),
            })],
            Role::Assistant => {
                let mut items = Vec::new();

                if !msg.content.is_empty() {
                    items.push(OpenAIInputItem::Message(OpenAIMessage {
                        role: "assistant".to_string(),
                        content: msg.content.clone(),
                    }));
                }

                if let Some(tool_calls) = &msg.tool_calls {
                    for tool_call in tool_calls {
                        items.push(OpenAIInputItem::FunctionCall(OpenAIFunctionCallInput {
                            call_id: tool_call.id.clone(),
                            name: tool_call.function.name.clone(),
                            arguments: tool_call.function.arguments.clone(),
                        }));
                    }
                }

                if items.is_empty() {
                    items.push(OpenAIInputItem::Message(OpenAIMessage {
                        role: "assistant".to_string(),
                        content: String::new(),
                    }));
                }

                items
            }
            Role::Tool => {
                vec![OpenAIInputItem::FunctionCallOutput(
                    OpenAIFunctionCallOutputInput {
                        call_id: msg.tool_call_id.clone().unwrap_or_default(),
                        output: msg.content.clone(),
                    },
                )]
            }
            Role::System => {
                vec![OpenAIInputItem::Message(OpenAIMessage {
                    role: "user".to_string(),
                    content: format!("[System]: {}", msg.content),
                })]
            }
        }
    }
}
