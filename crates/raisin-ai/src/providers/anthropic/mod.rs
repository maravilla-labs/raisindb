//! Anthropic API provider implementation.
//!
//! Uses Anthropic's Messages API (`/v1/messages`) for chat completions
//! with native tool use support.
//!
//! Supported models:
//! - claude-opus-4-5
//! - claude-sonnet-4-5
//! - claude-3-5-sonnet
//! - claude-3-5-haiku
//! - claude-3-haiku

#[cfg(test)]
mod tests;
mod trait_impl;
pub(crate) mod types;

use super::http_helpers::{build_client, SecretKey};
use super::structured_output::STRUCTURED_OUTPUT_TOOL;
use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::provider::{ProviderError, Result};
use crate::types::{CompletionRequest, CompletionResponse, ResponseFormat, Role};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use types::*;

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

/// Anthropic provider configuration.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    api_key: SecretKey,
    client: Client,
    base_url: String,
    cache: Arc<ModelCache>,
}

impl AnthropicProvider {
    /// Creates a new Anthropic provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: build_client(),
            base_url: ANTHROPIC_API_BASE.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Creates a new Anthropic provider with custom base URL.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: build_client(),
            base_url: base_url.into(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    // ── Model helpers ──────────────────────────────────────────────

    /// Returns a static list of known Anthropic models.
    ///
    /// Since Anthropic doesn't provide a models API endpoint, we maintain
    /// a curated list of available models with their capabilities.
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo::new("claude-opus-4-5", "Claude Opus 4.5")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(16384)
                .with_metadata(serde_json::json!({
                    "family": "claude-4",
                    "tier": "opus",
                    "release_date": "2025-01"
                })),
            ModelInfo::new("claude-sonnet-4-5", "Claude Sonnet 4.5")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(8192)
                .with_metadata(serde_json::json!({
                    "family": "claude-4",
                    "tier": "sonnet",
                    "release_date": "2025-01"
                })),
            ModelInfo::new("claude-3-5-sonnet-20241022", "Claude 3.5 Sonnet")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(8192)
                .with_metadata(serde_json::json!({
                    "family": "claude-3.5",
                    "tier": "sonnet",
                    "release_date": "2024-10"
                })),
            ModelInfo::new("claude-3-5-haiku-20241022", "Claude 3.5 Haiku")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(8192)
                .with_metadata(serde_json::json!({
                    "family": "claude-3.5",
                    "tier": "haiku",
                    "release_date": "2024-10"
                })),
            ModelInfo::new("claude-3-opus-20240229", "Claude 3 Opus")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(4096)
                .with_metadata(serde_json::json!({
                    "family": "claude-3",
                    "tier": "opus",
                    "release_date": "2024-02"
                })),
            ModelInfo::new("claude-3-sonnet-20240229", "Claude 3 Sonnet")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(4096)
                .with_metadata(serde_json::json!({
                    "family": "claude-3",
                    "tier": "sonnet",
                    "release_date": "2024-02"
                })),
            ModelInfo::new("claude-3-haiku-20240307", "Claude 3 Haiku")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(200000)
                .with_max_output_tokens(4096)
                .with_metadata(serde_json::json!({
                    "family": "claude-3",
                    "tier": "haiku",
                    "release_date": "2024-03"
                })),
        ]
    }

    /// Validates that the model is supported for chat.
    fn validate_chat_model(model: &str) -> Result<()> {
        const SUPPORTED_MODELS: &[&str] = &[
            "claude-opus-4-5",
            "claude-sonnet-4-5",
            "claude-3-5-sonnet",
            "claude-3-5-haiku",
            "claude-3-opus",
            "claude-3-sonnet",
            "claude-3-haiku",
        ];

        if SUPPORTED_MODELS.iter().any(|m| model.starts_with(m)) {
            Ok(())
        } else {
            Err(ProviderError::InvalidModel(format!(
                "Unsupported model: {}. Supported models: {}",
                model,
                SUPPORTED_MODELS.join(", ")
            )))
        }
    }

    // ── Request building ───────────────────────────────────────────

    /// Build an `AnthropicChatRequest` from a `CompletionRequest`.
    ///
    /// Handles system prompt extraction, message conversion, tool
    /// definitions, response format, and streaming flag.
    fn build_chat_request(request: &CompletionRequest, stream: bool) -> AnthropicChatRequest {
        let mut system_prompt = request.system.clone();
        let mut anthropic_messages = Vec::new();

        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    // Anthropic uses a separate system parameter
                    system_prompt = Some(msg.content.clone());
                }
                Role::User | Role::Assistant => {
                    let content = if let Some(tool_calls) = &msg.tool_calls {
                        let tool_use_content: Vec<AnthropicContent> = tool_calls
                            .iter()
                            .map(|call| AnthropicContent::ToolUse {
                                id: call.id.clone(),
                                name: call.function.name.clone(),
                                input: serde_json::from_str(&call.function.arguments)
                                    .unwrap_or_default(),
                            })
                            .collect();

                        if msg.content.is_empty() {
                            tool_use_content
                        } else {
                            let mut content = vec![AnthropicContent::Text {
                                text: msg.content.clone(),
                            }];
                            content.extend(tool_use_content);
                            content
                        }
                    } else {
                        vec![AnthropicContent::Text {
                            text: msg.content.clone(),
                        }]
                    };

                    let role_str = match msg.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        _ => "user",
                    };

                    anthropic_messages.push(AnthropicMessage {
                        role: role_str.to_string(),
                        content,
                    });
                }
                Role::Tool => {
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: vec![AnthropicContent::ToolResult {
                            tool_use_id: msg.tool_call_id.clone().unwrap_or_default(),
                            content: msg.content.clone(),
                        }],
                    });
                }
            }
        }

        let mut converted_tools = request.tools.as_ref().map(|tools| Self::convert_tools(tools));
        let mut tool_choice = None;
        Self::apply_response_format(
            request.response_format.as_ref(),
            &mut converted_tools,
            &mut tool_choice,
        );

        // Anthropic requires max_tokens to be specified
        let max_tokens = request.max_tokens.unwrap_or(4096);

        AnthropicChatRequest {
            model: request.model.clone(),
            messages: anthropic_messages,
            max_tokens,
            system: system_prompt,
            temperature: request.temperature,
            tools: converted_tools,
            tool_choice,
            stream: if stream { Some(true) } else { None },
        }
    }

    /// Send a JSON request to the Anthropic Messages API.
    ///
    /// Adds `x-api-key` and `anthropic-version` headers, and maps
    /// HTTP errors to `ProviderError`.
    async fn send_api_request(
        &self,
        body: &AnthropicChatRequest,
    ) -> Result<reqwest::Response> {
        use super::http_helpers;

        http_helpers::send_json_request(
            &self.client,
            &format!("{}/messages", self.base_url),
            ("x-api-key", self.api_key.expose().to_string()),
            body,
            &[("anthropic-version", ANTHROPIC_VERSION)],
            |r| Box::pin(http_helpers::handle_anthropic_error(r)),
        )
        .await
    }

    // ── Message / tool conversion ──────────────────────────────────

    /// Converts generic tool definitions to Anthropic format.
    fn convert_tools(tools: &[crate::types::ToolDefinition]) -> Vec<AnthropicTool> {
        tools
            .iter()
            .map(|tool| AnthropicTool {
                name: tool.function.name.clone(),
                description: tool.function.description.clone(),
                input_schema: tool.function.parameters.clone(),
            })
            .collect()
    }

    // ── Structured output (tool injection) ─────────────────────────

    /// Applies structured output settings from `ResponseFormat` to the request.
    ///
    /// - `Text` / `JsonObject`: no-op (Anthropic doesn't have a json_object mode;
    ///   for `JsonObject` we rely on a system prompt hint).
    /// - `JsonSchema`: injects a synthetic tool whose `input_schema` matches
    ///   the requested schema and forces the model to call it via `tool_choice`.
    fn apply_response_format(
        response_format: Option<&ResponseFormat>,
        tools: &mut Option<Vec<AnthropicTool>>,
        tool_choice: &mut Option<AnthropicToolChoice>,
    ) {
        let Some(format) = response_format else {
            return;
        };

        match format {
            ResponseFormat::Text | ResponseFormat::JsonObject => {}
            ResponseFormat::JsonSchema { schema } => {
                let tool_name = schema
                    .name
                    .as_deref()
                    .unwrap_or(STRUCTURED_OUTPUT_TOOL)
                    .to_string();

                let structured_tool = AnthropicTool {
                    name: tool_name.clone(),
                    description: "Respond with structured output matching the schema.".to_string(),
                    input_schema: schema.schema.clone(),
                };

                match tools {
                    Some(existing) => existing.push(structured_tool),
                    None => *tools = Some(vec![structured_tool]),
                }

                *tool_choice = Some(AnthropicToolChoice::Specific(
                    AnthropicToolChoiceSpecific {
                        choice_type: "tool".to_string(),
                        name: tool_name,
                    },
                ));
            }
        }
    }

    /// Checks whether the response contains a structured output tool call
    /// injected by `apply_response_format` and, if so, moves its JSON
    /// payload into `message.content`.
    fn extract_structured_output(
        response: &mut CompletionResponse,
        response_format: Option<&ResponseFormat>,
    ) {
        super::structured_output::extract_structured_output(response, response_format);
    }
}
