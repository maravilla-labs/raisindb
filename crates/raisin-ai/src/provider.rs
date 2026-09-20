//! Provider trait and common functionality.
//!
//! This module defines the core trait that all AI providers must implement,
//! as well as common utilities for working with providers.

use crate::model_cache::ModelInfo;
use crate::types::{
    CompletionRequest, CompletionResponse, FunctionCall, Message, StreamChunk, ToolCall,
};
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

/// Does this provider error describe a tool call rejected on its schema?
///
/// Shared by the two recoveries below so they cannot disagree about which
/// failures are recoverable. Authentication, transport, quota and ordinary
/// generation errors do not match.
fn is_tool_schema_rejection(message: &str) -> bool {
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
    names_tool_call && names_schema_problem
}

/// Recover a tool call the provider rejected for carrying UNDECLARED arguments.
///
/// A model given many similarly shaped tools will sometimes pad one call with
/// another tool's properties — `draft-function` arriving with `upsert-agent`'s
/// `provider`, `model` and `max_history_messages` set to their zero values is
/// the case this was written for. The call is otherwise complete and correct,
/// and the provider hands the whole rejected generation back to us, so there is
/// no need to ask the model again: the undeclared keys can only be dropped.
///
/// Asking again is what we used to do, and it does not work. The same model
/// that padded the call pads it again — an agent that cannot draft a function
/// stays unable to draft a function, having now paid for two completions.
///
/// This is deliberately conservative. It returns `None` unless ALL of:
///
/// * the error names a tool-schema rejection and carries a parseable failed
///   generation;
/// * the tool is one this request actually offered, with an object schema —
///   we never invent a call the model did not make, or one the caller did not
///   advertise;
/// * at least one argument is undeclared, so there is something to fix. A call
///   rejected for any other reason (a wrong type, a bad enum) is left to the
///   model, because dropping keys would not address it;
/// * every `required` property survives the drop. If removing the undeclared
///   keys would leave the call incomplete, the arguments were not merely padded
///   and guessing is worse than retrying.
pub fn salvage_rejected_tool_call(
    request: &CompletionRequest,
    error: &ProviderError,
) -> Option<CompletionResponse> {
    let ProviderError::RequestFailed(message) = error else {
        return None;
    };
    if !is_tool_schema_rejection(message) {
        return None;
    }
    let tools = request.tools.as_ref().filter(|tools| !tools.is_empty())?;

    let (_, generation) =
        message.split_once(crate::providers::http_helpers::FAILED_GENERATION_MARKER)?;
    let generation: serde_json::Value = serde_json::from_str(generation.trim()).ok()?;

    let name = generation.get("name")?.as_str()?;
    // Providers send the arguments either as an object or as a JSON string.
    let arguments = match generation.get("arguments")? {
        serde_json::Value::String(raw) => serde_json::from_str(raw).ok()?,
        other => other.clone(),
    };
    let arguments = arguments.as_object()?;

    let schema = &tools
        .iter()
        .find(|tool| tool.function.name == name)?
        .function
        .parameters;
    let declared = schema.get("properties")?.as_object()?;

    let (kept, dropped): (serde_json::Map<_, _>, Vec<&String>) = arguments.iter().fold(
        (serde_json::Map::new(), Vec::new()),
        |(mut kept, mut dropped), (key, value)| {
            if declared.contains_key(key) {
                kept.insert(key.clone(), value.clone());
            } else {
                dropped.push(key);
            }
            (kept, dropped)
        },
    );
    if dropped.is_empty() {
        return None;
    }

    let required = schema
        .get("required")
        .and_then(|required| required.as_array());
    if let Some(required) = required {
        for property in required {
            let property = property.as_str()?;
            if !kept.contains_key(property) {
                return None;
            }
        }
    }

    tracing::warn!(
        tool = name,
        dropped = ?dropped,
        "tool call rejected for undeclared arguments; salvaged by dropping them"
    );

    Some(CompletionResponse {
        message: Message::assistant("").with_tool_calls(vec![ToolCall {
            id: format!("salvaged_{}", uuid::Uuid::new_v4()),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: serde_json::Value::Object(kept).to_string(),
            },
            index: None,
        }]),
        model: request.model.clone(),
        usage: None,
        stop_reason: Some("tool_calls".to_string()),
    })
}

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

    if !is_tool_schema_rejection(message) {
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
///
/// A call rejected only for undeclared arguments is salvaged outright rather
/// than re-generated: it is deterministic, it costs nothing, and re-asking the
/// model reproduces the same padding.
pub async fn complete_with_tool_repair(
    provider: &dyn AIProviderTrait,
    request: CompletionRequest,
) -> Result<CompletionResponse> {
    match provider.complete(request.clone()).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Some(salvaged) = salvage_rejected_tool_call(&request, &error) {
                return Ok(salvaged);
            }
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
            // A salvaged call has nothing left to stream: emit it as the single
            // chunk the consumer would have accumulated anyway.
            if let Some(salvaged) = salvage_rejected_tool_call(&request, &error) {
                let chunk = StreamChunk {
                    delta: String::new(),
                    tool_calls: salvaged.message.tool_calls.clone(),
                    usage: None,
                    stop_reason: salvaged.stop_reason.clone(),
                    model: Some(salvaged.model.clone()),
                };
                return Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })));
            }
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
                        // Deliberately NOT salvageable: `slug` is required and
                        // absent, so dropping the undeclared `model` still
                        // leaves an incomplete call. That keeps these two tests
                        // on the prompt-repair path they exist to cover.
                        r#"{"name":"draft-function","arguments":{"model":"x"}}"#
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

    /// The real Studio Builder failure, from the server log: a correct
    /// `draft-function` call padded with `upsert-agent`'s properties at their
    /// zero values. Every declared argument is present and right; the call is
    /// rejected solely for the seven keys that do not belong to this tool.
    fn draft_function_request() -> CompletionRequest {
        CompletionRequest::new("openai/gpt-oss-120b".to_string(), vec![Message::user("go")])
            .with_tools(vec![ToolDefinition::function(
                "draft-function".to_string(),
                "Draft a Studio function".to_string(),
                serde_json::json!({
                    "type": "object",
                    "required": ["slug", "title", "language", "source"],
                    "properties": {
                        "slug": { "type": "string" },
                        "title": { "type": "string" },
                        "language": { "type": "string" },
                        "source": { "type": "string" },
                        "timeout_ms": { "type": "integer" }
                    }
                }),
            )])
    }

    fn padded_rejection() -> ProviderError {
        ProviderError::RequestFailed(format!(
            "Tool call validation failed: tool call validation failed: parameters for tool \
             draft-function did not match schema: errors: [additionalProperties 'model', \
             'max_history_messages' not allowed]{}{}",
            crate::providers::http_helpers::FAILED_GENERATION_MARKER,
            r#"{"name": "draft-function", "arguments": {"slug":"detect-emoji","title":"Detect Emoji","language":"starlark","source":"def handler(input):\n    return {}\n","timeout_ms":2000,"auto_compact":false,"max_conversation_tokens":0,"max_history_messages":0,"max_tokens":0,"temperature":0,"model":"","provider":""}}"#
        ))
    }

    #[test]
    fn a_padded_tool_call_is_salvaged_by_dropping_the_undeclared_keys() {
        let salvaged =
            salvage_rejected_tool_call(&draft_function_request(), &padded_rejection()).unwrap();

        let calls = salvaged.message.tool_calls.expect("a tool call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "draft-function");

        let args: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
        let args = args.as_object().unwrap();

        // The declared arguments survive, values untouched.
        assert_eq!(args["slug"], "detect-emoji");
        assert_eq!(args["language"], "starlark");
        assert_eq!(args["timeout_ms"], 2000);
        // The other tool's properties are gone.
        for undeclared in [
            "model",
            "provider",
            "temperature",
            "max_tokens",
            "max_history_messages",
            "max_conversation_tokens",
            "auto_compact",
        ] {
            assert!(!args.contains_key(undeclared), "{undeclared} survived");
        }
        assert_eq!(args.len(), 5);
    }

    /// Salvage must not fire when dropping the extras would leave the call
    /// incomplete -- that is a differently-wrong call, and the model should get
    /// the retry instead of us shipping a half-built one.
    #[test]
    fn a_call_missing_a_required_argument_is_not_salvaged() {
        let error = ProviderError::RequestFailed(format!(
            "Tool call validation failed: parameters for tool draft-function did not match schema{}{}",
            crate::providers::http_helpers::FAILED_GENERATION_MARKER,
            r#"{"name": "draft-function", "arguments": {"slug":"x","title":"T","language":"starlark","model":""}}"#
        ));
        assert!(salvage_rejected_tool_call(&draft_function_request(), &error).is_none());
    }

    /// Nothing undeclared means the rejection was about something else -- a bad
    /// type, a failed enum. Dropping keys would not address it.
    #[test]
    fn a_call_with_no_undeclared_arguments_is_left_to_the_model() {
        let error = ProviderError::RequestFailed(format!(
            "Tool call validation failed: parameters for tool draft-function did not match schema{}{}",
            crate::providers::http_helpers::FAILED_GENERATION_MARKER,
            r#"{"name": "draft-function", "arguments": {"slug":"x","title":"T","language":"starlark","source":"s"}}"#
        ));
        assert!(salvage_rejected_tool_call(&draft_function_request(), &error).is_none());
    }

    /// We never synthesise a call for a tool the request did not offer.
    #[test]
    fn a_call_naming_an_unoffered_tool_is_not_salvaged() {
        let error = ProviderError::RequestFailed(format!(
            "Tool call validation failed: parameters for tool upsert-agent did not match schema{}{}",
            crate::providers::http_helpers::FAILED_GENERATION_MARKER,
            r#"{"name": "upsert-agent", "arguments": {"slug":"x","nonsense":1}}"#
        ));
        assert!(salvage_rejected_tool_call(&draft_function_request(), &error).is_none());
    }

    /// An auth or transport failure is not a schema rejection.
    #[test]
    fn a_non_schema_error_is_not_salvaged() {
        assert!(salvage_rejected_tool_call(
            &draft_function_request(),
            &ProviderError::RequestFailed("Authentication failed".to_string()),
        )
        .is_none());
    }

    /// The padded call must never reach the provider a second time: salvage
    /// runs before the prompt-based repair, so exactly one request is made.
    #[tokio::test]
    async fn salvage_costs_no_second_completion() {
        struct PaddingProvider {
            calls: Mutex<usize>,
        }

        #[async_trait]
        impl AIProviderTrait for PaddingProvider {
            async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse> {
                *self.calls.lock().unwrap() += 1;
                Err(padded_rejection())
            }
            fn provider_name(&self) -> &str {
                "padding-test"
            }
        }

        let provider = PaddingProvider {
            calls: Mutex::new(0),
        };
        let response = complete_with_tool_repair(&provider, draft_function_request())
            .await
            .expect("salvaged rather than failed");

        assert_eq!(*provider.calls.lock().unwrap(), 1, "no repair round trip");
        assert!(response.message.tool_calls.is_some());
    }
}
