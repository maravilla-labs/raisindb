//! Provider trait and common functionality.
//!
//! This module defines the core trait that all AI providers must implement,
//! as well as common utilities for working with providers.

use crate::model_cache::ModelInfo;
use crate::types::{CompletionRequest, CompletionResponse, Message, StreamChunk};
use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;
use thiserror::Error;

/// Errors that can occur during provider operations.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("API request failed: {0}")]
    RequestFailed(String),

    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Authentication error: {0}")]
    AuthenticationError(String),

    #[error("Invalid model: {0}")]
    InvalidModel(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Provider not available: {0}")]
    ProviderNotAvailable(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Timeout error")]
    Timeout,

    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

pub type Result<T> = std::result::Result<T, ProviderError>;

const TOOL_REPAIR_MAX_ERROR_CHARS: usize = 4000;

/// Build one provider-independent correction turn for a rejected tool call.
/// Authentication, transport, quota, and ordinary generation errors are not
/// repairable here and pass through unchanged.
pub fn tool_validation_repair_request(
    request: &CompletionRequest,
    error: &ProviderError,
) -> Option<CompletionRequest> {
    let ProviderError::RequestFailed(message) = error else {
        return None;
    };
    if request.tools.as_ref().is_none_or(Vec::is_empty) {
        return None;
    }

    let lower = message.to_ascii_lowercase();
    let names_tool_call = lower.contains("tool call") || lower.contains("function call");
    let names_schema_problem = [
        "validation failed",
        "did not match schema",
        "invalid arguments",
        "invalid parameters",
        "schema validation",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if !names_tool_call || !names_schema_problem {
        return None;
    }

    let failed = message
        .split_once(crate::providers::http_helpers::FAILED_GENERATION_MARKER)
        .map(|(_, generation)| generation.trim())
        .unwrap_or("");
    let failed: String = failed.chars().take(TOOL_REPAIR_MAX_ERROR_CHARS).collect();
    let summary = message
        .split(crate::providers::http_helpers::FAILED_GENERATION_MARKER)
        .next()
        .unwrap_or(message);

    let mut retry = request.clone();
    retry.messages.push(Message::user(format!(
        "The previous tool call was rejected because its arguments did not match the tool's JSON schema. \
         Correct the arguments using the tool definitions already provided and call the intended tool again. \
         Do not explain the correction and do not add undeclared arguments.\n\
         Validation error: {summary}\n\
         Rejected call: {}",
        if failed.is_empty() { "unavailable" } else { &failed },
    )));
    Some(retry)
}

/// Complete with at most one self-correction pass for tool-schema rejection.
pub async fn complete_with_tool_repair(
    provider: &dyn AIProviderTrait,
    request: CompletionRequest,
) -> Result<CompletionResponse> {
    match provider.complete(request.clone()).await {
        Ok(response) => Ok(response),
        Err(error) => {
            let Some(repair) = tool_validation_repair_request(&request, &error) else {
                return Err(error);
            };
            tracing::warn!(
                provider = provider.provider_name(),
                model = %request.model,
                "tool call failed schema validation; retrying once with validation feedback"
            );
            provider.complete(repair).await
        }
    }
}

/// Start a stream with the same bounded repair used for normal completions.
/// Tool validation errors occur before a provider returns its stream, so no
/// partial user-visible response is discarded by this retry.
pub async fn stream_complete_with_tool_repair(
    provider: &dyn AIProviderTrait,
    request: CompletionRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
    match provider.stream_complete(request.clone()).await {
        Ok(stream) => Ok(stream),
        Err(error) => {
            let Some(repair) = tool_validation_repair_request(&request, &error) else {
                return Err(error);
            };
            tracing::warn!(
                provider = provider.provider_name(),
                model = %request.model,
                "streamed tool call failed schema validation; retrying once with validation feedback"
            );
            provider.stream_complete(repair).await
        }
    }
}

/// Refuse a request that carries images when this provider cannot send them.
///
/// # Why this exists rather than "the image is just ignored"
///
/// Every provider builds its own wire format, so image support is necessarily
/// per-provider. What is NOT acceptable is the shape that produced this
/// function: a provider whose message conversion reads `msg.content` — a
/// `String` — and therefore drops `content_parts` on the floor with no error
/// anywhere. The request succeeds, the model answers, and the answer is a
/// confident description of an image it was never shown. There is no log line,
/// no status code and no way for the caller to tell that apart from a working
/// vision call, which makes it the worst possible failure for a captioning
/// feature: it does not break, it just quietly invents.
///
/// So every provider that has not been taught to carry images calls this at the
/// top of its message conversion. Erroring costs nothing — nobody has a working
/// vision flow through those providers today, because the image never left the
/// process.
pub fn reject_unsupported_images(provider: &str, messages: &[crate::types::Message]) -> Result<()> {
    let count: usize = messages.iter().map(|m| m.image_parts().len()).sum();
    if count == 0 {
        return Ok(());
    }
    Err(ProviderError::UnsupportedOperation(format!(
        "the `{provider}` provider cannot send images: {count} image content \
         part(s) were supplied and would have been silently dropped. Providers \
         that carry images today: ollama, openai, anthropic."
    )))
}

/// Core trait for AI providers.
///
/// This trait defines the interface that all AI providers (OpenAI, Anthropic, etc.)
/// must implement. It provides a unified way to interact with different providers
/// regardless of their underlying API differences.
///
/// # Example Implementation
///
/// ```rust,ignore
/// use raisin_ai::provider::{AIProviderTrait, Result};
/// use raisin_ai::types::{CompletionRequest, CompletionResponse, Message};
/// use async_trait::async_trait;
///
/// struct MyProvider {
///     api_key: String,
///     endpoint: String,
/// }
///
/// #[async_trait]
/// impl AIProviderTrait for MyProvider {
///     async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
///         // Implement provider-specific logic here
///         todo!()
///     }
///
///     fn provider_name(&self) -> &str {
///         "my-provider"
///     }
///
///     fn supports_streaming(&self) -> bool {
///         true
///     }
/// }
/// ```
#[async_trait]
pub trait AIProviderTrait: Send + Sync {
    /// Performs a chat completion request.
    ///
    /// # Arguments
    ///
    /// * `request` - The completion request with messages and parameters
    ///
    /// # Returns
    ///
    /// The completion response from the provider
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails, the API key is invalid,
    /// the model is not available, or other provider-specific errors occur.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;

    /// Returns the name of this provider.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let name = provider.provider_name();
    /// assert_eq!(name, "openai");
    /// ```
    fn provider_name(&self) -> &str;

    /// Returns whether this provider supports streaming responses.
    ///
    /// Default implementation returns `false`. Override if streaming is supported.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Returns whether this provider supports tool/function calling.
    ///
    /// Default implementation returns `false`. Override if tools are supported.
    fn supports_tools(&self) -> bool {
        false
    }

    /// Returns the list of models available from this provider.
    ///
    /// Default implementation returns an empty vector. Override to provide
    /// the actual list of supported models.
    ///
    /// DEPRECATED: Use `list_available_models()` for dynamic model discovery.
    fn available_models(&self) -> Vec<String> {
        Vec::new()
    }

    /// Lists available models with detailed information.
    ///
    /// This method fetches the current list of available models from the provider,
    /// including model capabilities and metadata. Results should be cached to
    /// avoid excessive API calls.
    ///
    /// # Returns
    ///
    /// A list of available models with their capabilities and metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails or models cannot be fetched.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let models = provider.list_available_models().await?;
    /// for model in models {
    ///     println!("{}: {}", model.id, model.name);
    ///     println!("  Chat: {}", model.capabilities.chat);
    ///     println!("  Tools: {}", model.capabilities.tools);
    /// }
    /// ```
    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        // Default implementation converts legacy available_models() to ModelInfo
        Ok(self
            .available_models()
            .into_iter()
            .map(|id| {
                ModelInfo::new(id.clone(), id).with_capabilities(
                    crate::model_cache::ModelCapabilities {
                        chat: true,
                        streaming: self.supports_streaming(),
                        tools: self.supports_tools(),
                        embeddings: false,
                        vision: false,
                    },
                )
            })
            .collect())
    }

    /// Validates that the given model is supported by this provider.
    ///
    /// # Arguments
    ///
    /// * `model` - The model ID to validate
    ///
    /// # Returns
    ///
    /// `Ok(())` if the model is valid, or an error if not supported.
    fn validate_model(&self, model: &str) -> Result<()> {
        let models = self.available_models();
        if models.is_empty() {
            // If no models are specified, assume any model is valid
            return Ok(());
        }

        if models.iter().any(|m| m == model) {
            Ok(())
        } else {
            Err(ProviderError::InvalidModel(format!(
                "Model '{}' is not supported by provider '{}'",
                model,
                self.provider_name()
            )))
        }
    }

    /// Performs a streaming chat completion request.
    ///
    /// Returns a stream of `StreamChunk` items. Each chunk may contain
    /// a text delta, tool call data, or usage/stop information.
    ///
    /// The default implementation returns an error. Providers that support
    /// streaming should override this method.
    async fn stream_complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        Err(ProviderError::UnsupportedOperation(format!(
            "Provider '{}' does not support streaming",
            self.provider_name()
        )))
    }

    /// Generates an embedding vector for the given text.
    ///
    /// # Arguments
    ///
    /// * `text` - The text to generate an embedding for
    /// * `model` - The embedding model to use (e.g., "text-embedding-3-small" for OpenAI)
    ///
    /// # Returns
    ///
    /// A vector of f32 values representing the embedding
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The provider does not support embeddings
    /// - The API request fails
    /// - The model is invalid
    /// - Network errors occur
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let embedding = provider.generate_embedding("Hello world", "text-embedding-3-small").await?;
    /// assert_eq!(embedding.len(), 1536); // dimension depends on model
    /// ```
    async fn generate_embedding(&self, _text: &str, _model: &str) -> Result<Vec<f32>> {
        // Default implementation returns an error - providers that support embeddings
        // should override this method
        Err(ProviderError::UnsupportedOperation(format!(
            "Provider '{}' does not support embeddings",
            self.provider_name()
        )))
    }
}

/// Provider factory result type.
pub type ProviderResult<T> = Result<T>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, ToolDefinition};
    use std::sync::Mutex;

    struct MockProvider;

    #[async_trait]
    impl AIProviderTrait for MockProvider {
        async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
            Ok(CompletionResponse {
                message: Message::assistant("Mock response"),
                model: request.model,
                usage: None,
                stop_reason: Some("stop".to_string()),
            })
        }

        fn provider_name(&self) -> &str {
            "mock"
        }

        fn supports_streaming(&self) -> bool {
            true
        }

        fn available_models(&self) -> Vec<String> {
            vec!["model-1".to_string(), "model-2".to_string()]
        }
    }

    #[tokio::test]
    async fn test_mock_provider() {
        let provider = MockProvider;
        let request = CompletionRequest::new("model-1".to_string(), vec![Message::user("Hello")]);
        let response = provider.complete(request).await.unwrap();

        assert_eq!(response.message.content, "Mock response");
        assert_eq!(response.model, "model-1");
    }

    #[test]
    fn test_provider_capabilities() {
        let provider = MockProvider;
        assert_eq!(provider.provider_name(), "mock");
        assert!(provider.supports_streaming());
        assert!(!provider.supports_tools()); // Default implementation
    }

    #[test]
    fn test_model_validation() {
        let provider = MockProvider;

        // Valid models
        assert!(provider.validate_model("model-1").is_ok());
        assert!(provider.validate_model("model-2").is_ok());

        // Invalid model
        let result = provider.validate_model("invalid-model");
        assert!(matches!(result, Err(ProviderError::InvalidModel(_))));
    }

    fn tool_request() -> CompletionRequest {
        CompletionRequest::new("any-model".to_string(), vec![Message::user("Create it")])
            .with_tools(vec![ToolDefinition::function(
                "draft-function".to_string(),
                "Draft it".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": { "slug": { "type": "string" } },
                    "required": ["slug"],
                    "additionalProperties": false
                }),
            )])
    }

    struct RepairingProvider {
        requests: Mutex<Vec<CompletionRequest>>,
        always_fail: bool,
    }

    #[async_trait]
    impl AIProviderTrait for RepairingProvider {
        async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            if requests.len() == 1 || self.always_fail {
                return Err(ProviderError::RequestFailed(
                    concat!(
                        "Tool call validation failed: parameters did not match schema",
                        "\nFailed generation: ",
                        r#"{"name":"draft-function","arguments":{"slug":"demo","model":"x"}}"#
                    )
                    .to_string(),
                ));
            }
            Ok(CompletionResponse {
                message: Message::assistant("repaired"),
                model: request.model,
                usage: None,
                stop_reason: Some("stop".to_string()),
            })
        }

        fn provider_name(&self) -> &str {
            "provider-neutral-test"
        }
    }

    #[tokio::test]
    async fn test_shared_completion_repairs_tool_validation_once() {
        let provider = RepairingProvider {
            requests: Mutex::new(Vec::new()),
            always_fail: false,
        };
        let response = complete_with_tool_repair(&provider, tool_request())
            .await
            .unwrap();
        assert_eq!(response.message.content, "repaired");

        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let feedback = &requests[1].messages.last().unwrap().content;
        assert!(feedback.contains("did not match"));
        assert!(feedback.contains("do not add undeclared arguments"));
        assert!(feedback.contains("draft-function"));
        assert_eq!(requests[1].tools.as_ref().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_shared_completion_stops_after_one_failed_repair() {
        let provider = RepairingProvider {
            requests: Mutex::new(Vec::new()),
            always_fail: true,
        };
        assert!(complete_with_tool_repair(&provider, tool_request())
            .await
            .is_err());
        assert_eq!(provider.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_tool_repair_ignores_unrelated_errors() {
        assert!(
            tool_validation_repair_request(&tool_request(), &ProviderError::RateLimitExceeded,)
                .is_none()
        );
        assert!(tool_validation_repair_request(
            &tool_request(),
            &ProviderError::RequestFailed("Authentication failed".to_string()),
        )
        .is_none());
    }
}
