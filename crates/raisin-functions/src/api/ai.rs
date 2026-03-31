// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI API for functions to call LLM providers
//!
//! This module provides AI/LLM capabilities to JavaScript functions, allowing them
//! to call various AI providers (OpenAI, Anthropic, Ollama) for completions and chat.

use async_trait::async_trait;
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// AI provider implementation that can be injected into the function runtime
#[async_trait]
pub trait AIApi: Send + Sync {
    /// Call AI completion
    ///
    /// # Arguments
    ///
    /// * `request` - The completion request with messages, model, and parameters
    ///
    /// # Returns
    ///
    /// The completion response from the AI provider
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails, the model is not available,
    /// or the tenant doesn't have AI configured.
    async fn completion(&self, request: CompletionRequest) -> Result<CompletionResponse>;

    /// List available models
    ///
    /// # Returns
    ///
    /// A list of available models with their metadata
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails or no providers are configured
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    /// Get default model for a use case
    ///
    /// # Arguments
    ///
    /// * `use_case` - The use case (e.g., "chat", "completion", "agent")
    ///
    /// # Returns
    ///
    /// The model ID of the default model for the use case, if configured
    ///
    /// # Errors
    ///
    /// Returns an error if no default model is configured for the use case
    async fn get_default_model(&self, use_case: &str) -> Result<Option<String>>;
}

/// Completion request from JavaScript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The model to use (e.g., "gpt-4o", "claude-3-5-sonnet")
    pub model: String,

    /// The conversation messages
    pub messages: Vec<Message>,

    /// Optional system prompt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,

    /// Sampling temperature (0.0-2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Optional tool definitions for agentic workflows
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
}

/// A message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the message sender (user, assistant, system, tool)
    pub role: String,

    /// The text content of the message
    pub content: String,

    /// Optional tool calls made by the assistant
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,

    /// Optional tool call ID (for tool responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Optional name (for function/tool messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Completion response to JavaScript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The generated message
    pub message: Message,

    /// The model that generated the response
    pub model: String,

    /// Usage statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageStats>,

    /// Stop reason (e.g., "stop", "length", "tool_use")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Usage statistics for a completion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    /// Number of tokens in the prompt
    pub prompt_tokens: u32,

    /// Number of tokens in the completion
    pub completion_tokens: u32,

    /// Total tokens used
    pub total_tokens: u32,
}

/// Information about an available model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Unique model identifier (e.g., "gpt-4o")
    pub id: String,

    /// Human-readable display name
    pub name: String,

    /// The provider of this model
    pub provider: String,

    /// Use cases this model supports
    pub use_cases: Vec<String>,

    /// Model capabilities
    pub capabilities: ModelCapabilities,
}

/// Model capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Supports chat/conversation
    pub chat: bool,

    /// Supports streaming
    pub streaming: bool,

    /// Supports tool/function calling
    pub tools: bool,

    /// Supports vision/image inputs
    pub vision: bool,
}

/// Error type for AI API operations
#[derive(Debug, thiserror::Error)]
pub enum AIApiError {
    #[error("AI provider not configured for tenant")]
    NotConfigured,

    #[error("API request failed: {0}")]
    RequestFailed(String),

    #[error("Invalid model: {0}")]
    InvalidModel(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<AIApiError> for Error {
    fn from(err: AIApiError) -> Self {
        Error::Internal(err.to_string())
    }
}

/// Mock implementation for testing
pub struct MockAIApi;

#[async_trait]
impl AIApi for MockAIApi {
    async fn completion(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        Ok(CompletionResponse {
            message: Message {
                role: "assistant".to_string(),
                content: format!(
                    "Mock response to: {}",
                    request
                        .messages
                        .last()
                        .map(|m| m.content.as_str())
                        .unwrap_or("")
                ),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            model: request.model,
            usage: Some(UsageStats {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
            stop_reason: Some("stop".to_string()),
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4 Optimized".to_string(),
                provider: "openai".to_string(),
                use_cases: vec!["chat".to_string(), "completion".to_string()],
                capabilities: ModelCapabilities {
                    chat: true,
                    streaming: true,
                    tools: true,
                    vision: true,
                },
            },
            ModelInfo {
                id: "claude-3-5-sonnet".to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                use_cases: vec!["chat".to_string(), "agent".to_string()],
                capabilities: ModelCapabilities {
                    chat: true,
                    streaming: true,
                    tools: true,
                    vision: true,
                },
            },
        ])
    }

    async fn get_default_model(&self, use_case: &str) -> Result<Option<String>> {
        match use_case {
            "chat" | "completion" => Ok(Some("gpt-4o".to_string())),
            "agent" => Ok(Some("claude-3-5-sonnet".to_string())),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_ai_api_completion() {
        let api = MockAIApi;
        let request = CompletionRequest {
            model: "gpt-4o".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Hello!".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            system: None,
            temperature: None,
            max_tokens: None,
            tools: None,
        };

        let response = api.completion(request).await.unwrap();
        assert_eq!(response.message.role, "assistant");
        assert!(response.message.content.contains("Hello!"));
        assert_eq!(response.model, "gpt-4o");
        assert!(response.usage.is_some());
    }

    #[tokio::test]
    async fn test_mock_ai_api_list_models() {
        let api = MockAIApi;
        let models = api.list_models().await.unwrap();
        assert!(models.len() >= 2);
        assert!(models.iter().any(|m| m.id == "gpt-4o"));
        assert!(models.iter().any(|m| m.id == "claude-3-5-sonnet"));
    }

    #[tokio::test]
    async fn test_mock_ai_api_default_model() {
        let api = MockAIApi;

        let chat_model = api.get_default_model("chat").await.unwrap();
        assert_eq!(chat_model, Some("gpt-4o".to_string()));

        let agent_model = api.get_default_model("agent").await.unwrap();
        assert_eq!(agent_model, Some("claude-3-5-sonnet".to_string()));

        let unknown = api.get_default_model("unknown").await.unwrap();
        assert_eq!(unknown, None);
    }
}
