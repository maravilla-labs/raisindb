//! Tests for the OpenRouter provider.

use super::*;
use crate::provider::AIProviderTrait;
use crate::types::{FunctionCall, ToolCall};

#[test]
fn test_provider_creation() {
    let provider = OpenRouterProvider::new("test-key");
    assert_eq!(provider.provider_name(), "openrouter");
    assert_eq!(provider.api_key.expose(), "test-key");
    assert_eq!(provider.base_url, OPENROUTER_API_BASE);
    assert_eq!(provider.http_referer, DEFAULT_REFERER);
    assert_eq!(provider.app_name, DEFAULT_APP_NAME);
}

#[test]
fn test_provider_with_app_info() {
    let provider = OpenRouterProvider::with_app_info("test-key", "https://example.com", "MyApp");
    assert_eq!(provider.http_referer, "https://example.com");
    assert_eq!(provider.app_name, "MyApp");
}

#[test]
fn test_provider_capabilities() {
    let provider = OpenRouterProvider::new("test-key");
    assert_eq!(provider.provider_name(), "openrouter");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

#[test]
fn test_message_conversion() {
    let messages = vec![
        Message::system("You are helpful"),
        Message::user("Hello"),
        Message::assistant("Hi there"),
    ];

    let converted = OpenRouterProvider::convert_messages(&messages[1..]);
    assert_eq!(converted.len(), 2);
    assert_eq!(converted[0].role, "user");
    assert_eq!(converted[0].content, Some("Hello".to_string()));
    assert_eq!(converted[1].role, "assistant");
    assert_eq!(converted[1].content, Some("Hi there".to_string()));
}

#[test]
fn test_message_conversion_with_tools() {
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: "get_weather".to_string(),
            arguments: r#"{"location":"London"}"#.to_string(),
        },
        index: None,
    };

    let message = Message::assistant("").with_tool_calls(vec![tool_call]);
    let converted = OpenRouterProvider::convert_messages(&[message]);

    assert_eq!(converted.len(), 1);
    assert_eq!(converted[0].role, "assistant");
    assert!(converted[0].tool_calls.is_some());
    assert_eq!(converted[0].tool_calls.as_ref().unwrap().len(), 1);
}

#[test]
fn test_convert_openrouter_model() {
    let provider = OpenRouterProvider::new("test-key");
    let model = OpenRouterModel {
        id: "openai/gpt-4o".to_string(),
        name: Some("GPT-4 Omni".to_string()),
        context_length: Some(128000),
        pricing: OpenRouterPricing {
            prompt: "0.005".to_string(),
            completion: "0.015".to_string(),
        },
        architecture: Some(serde_json::json!({"modality": "text+vision"})),
    };

    let info = provider.convert_openrouter_model(model);
    assert_eq!(info.id, "openai/gpt-4o");
    assert_eq!(info.name, "GPT-4 Omni");
    assert_eq!(info.context_window, Some(128000));
    assert!(info.capabilities.chat);
    assert!(info.capabilities.vision);
    assert!(info.capabilities.tools);
    assert!(info.capabilities.streaming);
    assert!(!info.capabilities.embeddings);
}
