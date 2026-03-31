//! Shared HTTP error handling helpers for AI providers.
//!
//! This module provides common utilities for handling HTTP responses and errors
//! from various AI provider APIs. It reduces code duplication across providers
//! by centralizing error parsing and response handling logic.

use crate::provider::{ProviderError, Result};
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Default connect timeout for AI provider HTTP clients.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default request timeout for AI provider HTTP clients.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Creates an HTTP client with sensible timeouts for AI provider requests.
pub fn build_client() -> Client {
    Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .timeout(DEFAULT_REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// A wrapper for API keys that redacts the value in Debug and Display output.
#[derive(Clone)]
pub struct SecretKey(String);

impl SecretKey {
    /// Wrap a string as a secret key.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Access the raw key value (for use in HTTP headers).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(***)")
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Generic error detail structure used by OpenAI-compatible APIs.
/// This includes OpenAI, Groq, OpenRouter, and similar providers.
#[derive(Debug, serde::Deserialize)]
pub struct OpenAIStyleErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
    pub code: Option<String>,
    #[serde(default)]
    pub failed_generation: Option<String>,
}

/// Generic error wrapper for OpenAI-compatible APIs.
#[derive(Debug, serde::Deserialize)]
pub struct OpenAIStyleError {
    pub error: OpenAIStyleErrorDetail,
}

/// Anthropic-specific error detail structure.
#[derive(Debug, serde::Deserialize)]
pub struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

/// Anthropic error wrapper.
#[derive(Debug, serde::Deserialize)]
pub struct AnthropicError {
    pub error: AnthropicErrorDetail,
}

/// Gemini-specific error detail structure.
#[derive(Debug, serde::Deserialize)]
pub struct GeminiErrorDetail {
    pub message: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub code: Option<i32>,
}

/// Gemini error wrapper.
#[derive(Debug, serde::Deserialize)]
pub struct GeminiError {
    pub error: GeminiErrorDetail,
}

/// Azure OpenAI-specific error detail structure.
#[derive(Debug, serde::Deserialize)]
pub struct AzureErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
    pub code: Option<String>,
}

/// Azure error wrapper.
#[derive(Debug, serde::Deserialize)]
pub struct AzureError {
    pub error: AzureErrorDetail,
}

/// Simple error structure for providers like Ollama.
#[derive(Debug, serde::Deserialize)]
pub struct SimpleError {
    pub error: String,
}

/// Maps OpenAI-style error types to ProviderError.
///
/// This handles common error type strings like "authentication_error",
/// "rate_limit_error", and "invalid_request_error".
pub fn map_openai_style_error(error: OpenAIStyleError) -> ProviderError {
    let mut error_msg = error.error.message.clone();
    if let Some(failed_gen) = &error.error.failed_generation {
        error_msg.push_str("\nFailed generation: ");
        error_msg.push_str(failed_gen);
    }

    match error.error.error_type.as_deref() {
        Some("invalid_request_error") => ProviderError::RequestFailed(error_msg),
        Some("authentication_error") => ProviderError::InvalidApiKey,
        Some("rate_limit_error") => ProviderError::RateLimitExceeded,
        _ => ProviderError::RequestFailed(error_msg),
    }
}

/// Maps Anthropic error types to ProviderError.
pub fn map_anthropic_error(error: AnthropicError) -> ProviderError {
    match error.error.error_type.as_str() {
        "authentication_error" => ProviderError::InvalidApiKey,
        "rate_limit_error" => ProviderError::RateLimitExceeded,
        _ => ProviderError::RequestFailed(error.error.message),
    }
}

/// Maps Gemini error status codes to ProviderError.
pub fn map_gemini_error(error: GeminiError) -> ProviderError {
    match error.error.status.as_deref() {
        Some("INVALID_ARGUMENT") => ProviderError::RequestFailed(error.error.message),
        Some("UNAUTHENTICATED") => ProviderError::InvalidApiKey,
        Some("RESOURCE_EXHAUSTED") => ProviderError::RateLimitExceeded,
        Some("NOT_FOUND") => ProviderError::InvalidModel(error.error.message),
        _ => ProviderError::RequestFailed(error.error.message),
    }
}

/// Maps Azure OpenAI error codes to ProviderError.
pub fn map_azure_error(error: AzureError) -> ProviderError {
    match error.error.code.as_deref() {
        Some("invalid_request_error") => ProviderError::RequestFailed(error.error.message),
        Some("401") | Some("Unauthorized") => ProviderError::InvalidApiKey,
        Some("429") | Some("RateLimitExceeded") => ProviderError::RateLimitExceeded,
        Some("DeploymentNotFound") => ProviderError::InvalidModel(error.error.message),
        _ => ProviderError::RequestFailed(error.error.message),
    }
}

/// Creates a generic HTTP error message for when structured error parsing fails.
pub fn create_http_error(status: reqwest::StatusCode, error_text: &str) -> ProviderError {
    ProviderError::RequestFailed(format!("HTTP {}: {}", status, error_text))
}

/// Handles HTTP error responses for OpenAI-style APIs.
///
/// This function attempts to parse the error response as an OpenAI-style error,
/// falling back to a generic HTTP error if parsing fails.
pub async fn handle_openai_style_error(response: Response) -> ProviderError {
    let status = response.status();
    let error_text = response.text().await.unwrap_or_default();

    if let Ok(error) = serde_json::from_str::<OpenAIStyleError>(&error_text) {
        map_openai_style_error(error)
    } else {
        create_http_error(status, &error_text)
    }
}

/// Handles HTTP error responses for Anthropic API.
pub async fn handle_anthropic_error(response: Response) -> ProviderError {
    let status = response.status();
    let error_text = response.text().await.unwrap_or_default();

    if let Ok(error) = serde_json::from_str::<AnthropicError>(&error_text) {
        map_anthropic_error(error)
    } else {
        create_http_error(status, &error_text)
    }
}

/// Handles HTTP error responses for Gemini API.
pub async fn handle_gemini_error(response: Response) -> ProviderError {
    let status = response.status();
    let error_text = response.text().await.unwrap_or_default();

    if let Ok(error) = serde_json::from_str::<GeminiError>(&error_text) {
        map_gemini_error(error)
    } else {
        create_http_error(status, &error_text)
    }
}

/// Handles HTTP error responses for Azure OpenAI API.
pub async fn handle_azure_error(response: Response) -> ProviderError {
    let status = response.status();
    let error_text = response.text().await.unwrap_or_default();

    if let Ok(error) = serde_json::from_str::<AzureError>(&error_text) {
        map_azure_error(error)
    } else {
        create_http_error(status, &error_text)
    }
}

/// Handles HTTP error responses for simple error formats (like Ollama).
pub async fn handle_simple_error(response: Response) -> ProviderError {
    let status = response.status();
    let error_text = response.text().await.unwrap_or_default();

    if let Ok(error) = serde_json::from_str::<SimpleError>(&error_text) {
        ProviderError::RequestFailed(error.error)
    } else {
        create_http_error(status, &error_text)
    }
}

/// Parses a successful JSON response, converting deserialization errors to ProviderError.
pub async fn parse_json_response<T: DeserializeOwned>(response: Response) -> Result<T> {
    response
        .json()
        .await
        .map_err(|e| ProviderError::DeserializationError(e.to_string()))
}

/// Helper to handle network errors from reqwest.
pub fn handle_network_error(e: reqwest::Error) -> ProviderError {
    ProviderError::NetworkError(e.to_string())
}

/// Creates a "Failed to fetch models" error message for model listing endpoints.
pub fn create_fetch_models_error(status: reqwest::StatusCode, error_text: &str) -> ProviderError {
    ProviderError::RequestFailed(format!(
        "Failed to fetch models: HTTP {}: {}",
        status, error_text
    ))
}

/// Checks if a response status is successful and returns the response if so,
/// or handles the error using the provided error handler.
pub async fn check_response<F>(response: Response, error_handler: F) -> Result<Response>
where
    F: FnOnce(Response) -> Pin<Box<dyn Future<Output = ProviderError> + Send>>,
{
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(error_handler(response).await)
    }
}

/// Sends a JSON POST request with authentication and error handling.
///
/// This generic helper reduces boilerplate across providers by handling:
/// - JSON serialization of the request body
/// - Authentication via a configurable header
/// - Optional extra headers (e.g., `anthropic-version`)
/// - Network error mapping
/// - HTTP error response mapping via a provider-specific handler
///
/// # Arguments
///
/// * `client` - The reqwest HTTP client
/// * `url` - The endpoint URL
/// * `auth_header` - A `(name, value)` pair for the auth header
///   (e.g., `("Authorization", "Bearer sk-...")` or `("x-api-key", "...")`)
/// * `body` - The request body to serialize as JSON
/// * `extra_headers` - Additional headers to include
/// * `error_handler` - A function that maps error responses to `ProviderError`
pub async fn send_json_request(
    client: &Client,
    url: &str,
    auth_header: (&str, String),
    body: &impl Serialize,
    extra_headers: &[(&str, &str)],
    error_handler: fn(Response) -> Pin<Box<dyn Future<Output = ProviderError> + Send>>,
) -> Result<Response> {
    let mut request = client
        .post(url)
        .header(auth_header.0, auth_header.1)
        .header("Content-Type", "application/json");

    for (name, value) in extra_headers {
        request = request.header(*name, *value);
    }

    let response = request
        .json(body)
        .send()
        .await
        .map_err(handle_network_error)?;

    if response.status().is_success() {
        Ok(response)
    } else {
        Err(error_handler(response).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_openai_style_error_auth() {
        let error = OpenAIStyleError {
            error: OpenAIStyleErrorDetail {
                message: "Invalid API key".to_string(),
                error_type: Some("authentication_error".to_string()),
                code: None,
                failed_generation: None,
            },
        };
        assert!(matches!(
            map_openai_style_error(error),
            ProviderError::InvalidApiKey
        ));
    }

    #[test]
    fn test_map_openai_style_error_rate_limit() {
        let error = OpenAIStyleError {
            error: OpenAIStyleErrorDetail {
                message: "Rate limit exceeded".to_string(),
                error_type: Some("rate_limit_error".to_string()),
                code: None,
                failed_generation: None,
            },
        };
        assert!(matches!(
            map_openai_style_error(error),
            ProviderError::RateLimitExceeded
        ));
    }

    #[test]
    fn test_map_openai_style_error_with_failed_generation() {
        let error = OpenAIStyleError {
            error: OpenAIStyleErrorDetail {
                message: "Invalid request".to_string(),
                error_type: Some("invalid_request_error".to_string()),
                code: None,
                failed_generation: Some("partial output here".to_string()),
            },
        };
        if let ProviderError::RequestFailed(msg) = map_openai_style_error(error) {
            assert!(msg.contains("Invalid request"));
            assert!(msg.contains("Failed generation:"));
            assert!(msg.contains("partial output here"));
        } else {
            panic!("Expected RequestFailed error");
        }
    }

    #[test]
    fn test_map_anthropic_error_auth() {
        let error = AnthropicError {
            error: AnthropicErrorDetail {
                error_type: "authentication_error".to_string(),
                message: "Invalid API key".to_string(),
            },
        };
        assert!(matches!(
            map_anthropic_error(error),
            ProviderError::InvalidApiKey
        ));
    }

    #[test]
    fn test_map_anthropic_error_rate_limit() {
        let error = AnthropicError {
            error: AnthropicErrorDetail {
                error_type: "rate_limit_error".to_string(),
                message: "Too many requests".to_string(),
            },
        };
        assert!(matches!(
            map_anthropic_error(error),
            ProviderError::RateLimitExceeded
        ));
    }

    #[test]
    fn test_map_gemini_error_unauthenticated() {
        let error = GeminiError {
            error: GeminiErrorDetail {
                message: "API key invalid".to_string(),
                status: Some("UNAUTHENTICATED".to_string()),
                code: None,
            },
        };
        assert!(matches!(
            map_gemini_error(error),
            ProviderError::InvalidApiKey
        ));
    }

    #[test]
    fn test_map_gemini_error_not_found() {
        let error = GeminiError {
            error: GeminiErrorDetail {
                message: "Model not found".to_string(),
                status: Some("NOT_FOUND".to_string()),
                code: None,
            },
        };
        assert!(matches!(
            map_gemini_error(error),
            ProviderError::InvalidModel(_)
        ));
    }

    #[test]
    fn test_map_azure_error_unauthorized() {
        let error = AzureError {
            error: AzureErrorDetail {
                message: "Unauthorized".to_string(),
                error_type: None,
                code: Some("401".to_string()),
            },
        };
        assert!(matches!(
            map_azure_error(error),
            ProviderError::InvalidApiKey
        ));
    }

    #[test]
    fn test_map_azure_error_deployment_not_found() {
        let error = AzureError {
            error: AzureErrorDetail {
                message: "Deployment gpt-5 not found".to_string(),
                error_type: None,
                code: Some("DeploymentNotFound".to_string()),
            },
        };
        assert!(matches!(
            map_azure_error(error),
            ProviderError::InvalidModel(_)
        ));
    }

    #[test]
    fn test_create_http_error() {
        let error = create_http_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "Server error");
        if let ProviderError::RequestFailed(msg) = error {
            assert!(msg.contains("HTTP 500"));
            assert!(msg.contains("Server error"));
        } else {
            panic!("Expected RequestFailed error");
        }
    }
}
