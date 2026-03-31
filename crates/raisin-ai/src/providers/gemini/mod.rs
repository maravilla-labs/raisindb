//! Google Gemini API provider implementation.
//!
//! Provides chat completions and tool calling using Google's Gemini API.
//!
//! Supported models:
//! - Chat: gemini-2.0-flash-exp, gemini-1.5-pro, gemini-1.5-flash, gemini-1.5-flash-8b
//! - All models support tool calling

#[cfg(test)]
mod tests;
mod trait_impl;
pub(crate) mod types;

use super::http_helpers::SecretKey;
use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::provider::{ProviderError, Result};
use crate::types::Message;
use crate::types::Role;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use types::*;

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

/// Google Gemini provider configuration
#[derive(Debug, Clone)]
pub struct GeminiProvider {
    api_key: SecretKey,
    client: Client,
    base_url: String,
    cache: Arc<ModelCache>,
}

impl GeminiProvider {
    /// Creates a new Gemini provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: GEMINI_API_BASE.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Creates a new Gemini provider with custom base URL
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            base_url: base_url.into(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Fetches the list of available models from Gemini API
    async fn fetch_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/models?key={}", self.base_url, self.api_key.expose()))
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

        let models_response: GeminiModelsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Convert Gemini models to our ModelInfo format
        let models = models_response
            .models
            .into_iter()
            .filter_map(|model| {
                // Only include generative models (gemini-*)
                if model.name.contains("gemini") {
                    Some(self.convert_gemini_model(model))
                } else {
                    None
                }
            })
            .collect();

        Ok(models)
    }

    /// Converts a Gemini model to our ModelInfo format
    fn convert_gemini_model(&self, model: GeminiModel) -> ModelInfo {
        // Extract model ID from full name (models/gemini-1.5-pro -> gemini-1.5-pro)
        let model_id = model
            .name
            .strip_prefix("models/")
            .unwrap_or(&model.name)
            .to_string();

        // All Gemini 1.5+ and 2.0+ models support tools
        let supports_tools = model_id.contains("gemini-1.5") || model_id.contains("gemini-2");

        // Vision support for pro and flash models
        let supports_vision = model_id.contains("pro") || model_id.contains("flash");

        let capabilities = ModelCapabilities {
            chat: true,
            embeddings: model_id.contains("embedding"),
            vision: supports_vision,
            tools: supports_tools,
            streaming: true,
        };

        ModelInfo::new(model_id, model.display_name.clone())
            .with_capabilities(capabilities)
            .with_context_window(model.input_token_limit.unwrap_or(32768) as u32)
            .with_max_output_tokens(model.output_token_limit.unwrap_or(8192) as u32)
            .with_metadata(serde_json::json!({
                "description": model.description,
                "version": model.version,
                "supported_generation_methods": model.supported_generation_methods,
            }))
    }

    /// Converts our Message type to Gemini Content format
    fn convert_messages_to_contents(messages: &[Message]) -> Vec<GeminiContent> {
        let mut contents = Vec::new();

        for msg in messages {
            match msg.role {
                Role::User => {
                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart::Text {
                            text: msg.content.clone(),
                        }],
                    });
                }
                Role::Assistant => {
                    let mut parts = Vec::new();

                    // Add text content if present
                    if !msg.content.is_empty() {
                        parts.push(GeminiPart::Text {
                            text: msg.content.clone(),
                        });
                    }

                    // Add function calls if present
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            let args: serde_json::Value =
                                serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                            parts.push(GeminiPart::FunctionCall {
                                function_call: GeminiFunctionCall {
                                    name: tc.function.name.clone(),
                                    args,
                                },
                            });
                        }
                    }

                    contents.push(GeminiContent {
                        role: "model".to_string(),
                        parts,
                    });
                }
                Role::Tool => {
                    // Tool results in Gemini format
                    let result: serde_json::Value = serde_json::from_str(&msg.content)
                        .unwrap_or_else(|_| serde_json::json!({ "result": msg.content }));

                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart::FunctionResponse {
                            function_response: GeminiFunctionResponse {
                                name: msg.name.clone().unwrap_or_default(),
                                response: result,
                            },
                        }],
                    });
                }
                Role::System => {
                    // System messages are handled separately via system_instruction
                }
            }
        }

        contents
    }

    /// Extracts system prompt from messages
    fn extract_system_prompt(
        messages: &[Message],
        request_system: Option<&String>,
    ) -> Option<GeminiContent> {
        // First check request.system, then look for system messages
        if let Some(system) = request_system {
            return Some(GeminiContent {
                role: "user".to_string(), // system_instruction uses "user" role
                parts: vec![GeminiPart::Text {
                    text: system.clone(),
                }],
            });
        }

        // Find system messages in the conversation
        for msg in messages {
            if msg.role == Role::System {
                return Some(GeminiContent {
                    role: "user".to_string(),
                    parts: vec![GeminiPart::Text {
                        text: msg.content.clone(),
                    }],
                });
            }
        }

        None
    }

    /// Converts our ToolDefinition to Gemini FunctionDeclaration
    fn convert_tools(tools: &Option<Vec<crate::types::ToolDefinition>>) -> Option<Vec<GeminiTool>> {
        tools.as_ref().map(|tool_defs| {
            let declarations: Vec<GeminiFunctionDeclaration> = tool_defs
                .iter()
                .map(|tool| GeminiFunctionDeclaration {
                    name: tool.function.name.clone(),
                    description: tool.function.description.clone(),
                    parameters: tool.function.parameters.clone(),
                })
                .collect();

            vec![GeminiTool {
                function_declarations: declarations,
            }]
        })
    }
}
