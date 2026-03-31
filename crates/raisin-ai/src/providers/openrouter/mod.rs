//! OpenRouter API provider implementation.
//!
//! OpenRouter provides access to multiple AI models through a unified API.
//! It uses an OpenAI-compatible API format with additional routing headers.
//!
//! Supported model families:
//! - openai/gpt-4o, openai/gpt-4-turbo
//! - anthropic/claude-3-sonnet, anthropic/claude-3-opus
//! - meta-llama/llama-3.3-70b-instruct
//! - google/gemini-pro
//! - And many more from various providers

#[cfg(test)]
mod tests;
mod trait_impl;
pub(crate) mod types;

use super::http_helpers::SecretKey;
use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::provider::{ProviderError, Result};
use crate::types::{Message, Role};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use types::*;

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour
const DEFAULT_REFERER: &str = "https://raisindb.com";
const DEFAULT_APP_NAME: &str = "RaisinDB";

/// OpenRouter provider configuration
#[derive(Debug, Clone)]
pub struct OpenRouterProvider {
    api_key: SecretKey,
    client: Client,
    base_url: String,
    cache: Arc<ModelCache>,
    http_referer: String,
    app_name: String,
}

impl OpenRouterProvider {
    /// Creates a new OpenRouter provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: OPENROUTER_API_BASE.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            http_referer: DEFAULT_REFERER.to_string(),
            app_name: DEFAULT_APP_NAME.to_string(),
        }
    }

    /// Creates a new OpenRouter provider with custom referer and app name
    pub fn with_app_info(
        api_key: impl Into<String>,
        http_referer: impl Into<String>,
        app_name: impl Into<String>,
    ) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: OPENROUTER_API_BASE.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            http_referer: http_referer.into(),
            app_name: app_name.into(),
        }
    }

    /// Creates a new OpenRouter provider with custom base URL (for testing)
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: base_url.into(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            http_referer: DEFAULT_REFERER.to_string(),
            app_name: DEFAULT_APP_NAME.to_string(),
        }
    }

    /// Fetches the list of available models from OpenRouter API
    async fn fetch_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key.expose()))
            .header("HTTP-Referer", &self.http_referer)
            .header("X-Title", &self.app_name)
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

        let models_response: OpenRouterModelsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Convert OpenRouter models to our ModelInfo format
        let models = models_response
            .data
            .into_iter()
            .map(|model| self.convert_openrouter_model(model))
            .collect();

        Ok(models)
    }

    /// Converts an OpenRouter model to our ModelInfo format
    fn convert_openrouter_model(&self, model: OpenRouterModel) -> ModelInfo {
        let capabilities = ModelCapabilities {
            chat: true,        // All OpenRouter models support chat
            embeddings: false, // OpenRouter doesn't provide embeddings through this API
            vision: model.id.contains("vision") || model.id.contains("gpt-4o"),
            tools: true,     // Most OpenRouter models support tools via OpenAI format
            streaming: true, // OpenRouter supports streaming
        };

        ModelInfo::new(
            model.id.clone(),
            model.name.unwrap_or_else(|| model.id.clone()),
        )
        .with_capabilities(capabilities)
        .with_context_window(model.context_length.unwrap_or(4096) as u32)
        .with_metadata(serde_json::json!({
            "pricing": {
                "prompt": model.pricing.prompt,
                "completion": model.pricing.completion,
            },
            "architecture": model.architecture,
        }))
    }

    /// Converts our Message type to OpenAI-compatible chat format
    fn convert_messages(messages: &[Message]) -> Vec<OpenAIChatMessage> {
        messages
            .iter()
            .map(|msg| match msg.role {
                Role::User => OpenAIChatMessage {
                    role: "user".to_string(),
                    content: Some(msg.content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                },
                Role::Assistant => OpenAIChatMessage {
                    role: "assistant".to_string(),
                    content: if msg.content.is_empty() {
                        None
                    } else {
                        Some(msg.content.clone())
                    },
                    tool_calls: msg.tool_calls.as_ref().map(|calls| {
                        calls
                            .iter()
                            .map(|call| OpenAIToolCall {
                                id: call.id.clone(),
                                call_type: call.call_type.clone(),
                                function: OpenAIFunctionCall {
                                    name: call.function.name.clone(),
                                    arguments: call.function.arguments.clone(),
                                },
                            })
                            .collect()
                    }),
                    tool_call_id: None,
                },
                Role::Tool => OpenAIChatMessage {
                    role: "tool".to_string(),
                    content: Some(msg.content.clone()),
                    tool_calls: None,
                    tool_call_id: msg.tool_call_id.clone(),
                },
                Role::System => OpenAIChatMessage {
                    role: "system".to_string(),
                    content: Some(msg.content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                },
            })
            .collect()
    }
}
