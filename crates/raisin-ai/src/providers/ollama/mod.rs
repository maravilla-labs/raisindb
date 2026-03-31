//! Ollama local AI provider implementation.
//!
//! Supports chat completions using locally-hosted Ollama models.
//!
//! Supported chat models:
//! - llama3.3
//! - mistral
//! - qwen2.5
//! - (any other models installed in Ollama)

mod api_types;
mod model_discovery;
mod provider_impl;

#[cfg(test)]
mod tests;

use super::http_helpers::SecretKey;
use crate::model_cache::{ModelCache, ModelInfo};
use crate::provider::{ProviderError, Result};
use crate::types::{Message, Role};
use api_types::*;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

pub(crate) const OLLAMA_DEFAULT_BASE: &str = "http://localhost:11434/api";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Models known to support tool calling properly
/// Based on Ollama blog (https://ollama.com/blog/tool-support) and testing as of 2024-2025
/// Note: Model names are matched using prefix/contains logic to handle variants
///
/// IMPORTANT: llama3.2/llama3.3 are NOT included because Ollama's default template
/// forces them to always call tools regardless of context. See:
/// - https://github.com/ollama/ollama/issues/6127
/// - https://github.com/ollama/ollama/issues/9947
///   Use llama3.1 or llama3-groq-tool-use instead.
pub(crate) const TOOL_CAPABLE_MODELS: &[&str] = &[
    // Llama 3.1 - officially supported by Ollama for tool calling
    "llama3.1",
    // Groq-trained Llama with proper tool decision logic
    "llama3-groq-tool-use",
    // Qwen models handle tools well
    "qwen3",
    "qwen2.5",
    "qwen2.5-coder",
    // Mistral family
    "mistral",
    "mistral-nemo",
    "mixtral",
    // Google Gemma
    "gemma2",
    // Cohere Command-R
    "command-r",
    "command-r-plus",
    // Fireworks FireFunction
    "firefunction",
    // NousResearch Hermes models with tool support
    "hermes",
    "hermes3",
    // Nexusflow Athene
    "athene",
    // Microsoft Phi-4 (has tool calling capability)
    "phi4",
    // IBM Granite models
    "granite",
    // DeepSeek with tool support (custom variants)
    "deepseek-r1-tool",
    "deepseek-coder",
    // Alibaba models
    "marco",
];

/// Ollama provider configuration
#[derive(Debug, Clone)]
pub struct OllamaProvider {
    pub(crate) client: Client,
    pub(crate) base_url: String,
    /// Optional API key for authenticated Ollama endpoints
    pub(crate) api_key: Option<SecretKey>,
    pub(crate) cache: Arc<ModelCache>,
    /// Cache for model tool support (model_name -> supports_tools)
    pub(crate) tool_support_cache: Arc<RwLock<HashMap<String, bool>>>,
}

impl OllamaProvider {
    /// Creates a new Ollama provider with default endpoint (localhost:11434)
    pub fn new() -> Self {
        Self {
            client: super::http_helpers::build_client(),
            base_url: OLLAMA_DEFAULT_BASE.to_string(),
            api_key: None,
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            tool_support_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Creates a new Ollama provider with custom base URL
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: super::http_helpers::build_client(),
            base_url: base_url.into(),
            api_key: None,
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            tool_support_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Sets the API key for authenticated Ollama endpoints
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(SecretKey::new(api_key));
        self
    }

    /// Helper to add authorization header if API key is configured
    pub(crate) fn add_auth_header(
        &self,
        request: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            request.header("Authorization", format!("Bearer {}", key.expose()))
        } else {
            request
        }
    }

    /// Tests connection to Ollama endpoint
    ///
    /// 1. Fetches models from /tags to verify endpoint is reachable
    /// 2. If API key is configured, also tests /chat to verify auth works
    pub async fn test_connection(&self) -> Result<Vec<ModelInfo>> {
        // First, fetch models to verify endpoint is reachable
        let models = self.fetch_models().await?;

        // If API key is configured, verify chat endpoint works with auth
        if self.api_key.is_some() && !models.is_empty() {
            self.test_chat_auth(&models[0].id).await?;
        }

        Ok(models)
    }

    /// Tests that chat endpoint works with authentication
    async fn test_chat_auth(&self, model: &str) -> Result<()> {
        // Send minimal chat request to verify auth works
        let test_request = OllamaChatRequest {
            model: model.to_string(),
            messages: vec![OllamaMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                images: None,
                tool_calls: None,
            }],
            tools: None,
            stream: Some(false),
            options: Some(OllamaOptions {
                temperature: None,
                num_predict: Some(1), // Minimal tokens to make it fast
            }),
            format: None,
        };

        let request = self
            .client
            .post(format!("{}/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&test_request);
        let request = self.add_auth_header(request);
        let response = request
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::AuthenticationError(format!(
                "Chat endpoint authentication failed: HTTP {}: {}",
                status, error_text
            )));
        }

        Ok(())
    }

    /// Checks if a model supports tool calling based on its name
    /// Uses pattern matching against known tool-capable models
    pub(crate) fn model_supports_tools_by_name(model: &str) -> bool {
        // Extract base model name without version tags (e.g., "llama3.2:8b" -> "llama3.2")
        let base_name = model.split(':').next().unwrap_or(model).to_lowercase();

        // Check if the model name starts with any known tool-capable model
        TOOL_CAPABLE_MODELS.iter().any(|&capable_model| {
            base_name.starts_with(capable_model) || base_name.contains(capable_model)
        })
    }

    /// Checks if a model supports tool calling (with caching)
    pub(crate) async fn check_tool_support(&self, model: &str) -> bool {
        // Check cache first
        {
            let cache = self.tool_support_cache.read().await;
            if let Some(&supports) = cache.get(model) {
                return supports;
            }
        }

        // Determine support based on model name
        let supports = Self::model_supports_tools_by_name(model);

        // Cache the result
        {
            let mut cache = self.tool_support_cache.write().await;
            cache.insert(model.to_string(), supports);
        }

        supports
    }

    /// Converts our Message type to Ollama format
    pub(crate) fn convert_message(msg: &Message) -> OllamaMessage {
        // Extract first image from multimodal content (Ollama supports multiple, but we start with one)
        let images = msg
            .first_image()
            .map(|(image_data, _media_type)| vec![image_data.to_string()]);

        OllamaMessage {
            role: match msg.role {
                Role::User => "user".to_string(),
                Role::Assistant => "assistant".to_string(),
                Role::System => "system".to_string(),
                Role::Tool => "tool".to_string(),
            },
            content: msg.effective_text(), // Use effective_text() to get text from multimodal
            images,                        // Pass extracted images
            tool_calls: msg.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|call| OllamaToolCall {
                        function: OllamaFunctionCall {
                            name: call.function.name.clone(),
                            arguments: serde_json::from_str(&call.function.arguments)
                                .unwrap_or_default(),
                        },
                    })
                    .collect()
            }),
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}
