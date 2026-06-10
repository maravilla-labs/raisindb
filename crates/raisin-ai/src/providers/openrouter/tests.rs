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

// ── Streaming SSE parsing ─────────────────────────────────────────

use super::trait_impl::parse_openrouter_sse_events;

#[test]
fn test_parse_sse_text_delta() {
    let sse = r#"data: {"id":"gen-1","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}],"model":"openai/gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_openrouter_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "Hello");
}

#[test]
fn test_parse_sse_usage_on_final_chunk_with_null_finish() {
    // OpenRouter usage accounting: the final SSE chunk carries usage but may
    // have finish_reason null and an empty content delta.
    let sse = r#"data: {"id":"gen-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"model":"openai/gpt-4o"}

data: {"id":"gen-1","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}],"usage":{"prompt_tokens":30,"completion_tokens":12,"total_tokens":42},"model":"openai/gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_openrouter_sse_events(sse);
    assert_eq!(chunks.len(), 2);

    assert_eq!(
        chunks[0].as_ref().unwrap().stop_reason.as_deref(),
        Some("stop")
    );

    let usage_chunk = chunks[1].as_ref().unwrap();
    assert!(usage_chunk.stop_reason.is_none());
    let usage = usage_chunk.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 30);
    assert_eq!(usage.completion_tokens, 12);
    assert_eq!(usage.total_tokens, 42);
}

#[test]
fn test_parse_sse_usage_on_trailing_empty_choices_chunk() {
    // OpenAI stream_options style: trailing chunk with empty choices.
    let sse = r#"data: {"id":"gen-1","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":3,"total_tokens":14},"model":"openai/gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_openrouter_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    let c = chunks[0].as_ref().unwrap();
    assert!(c.stop_reason.is_none());
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 14);
}

#[test]
fn test_parse_sse_usage_on_finish_chunk_not_duplicated() {
    let sse = r#"data: {"id":"gen-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"model":"openai/gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_openrouter_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 15);
}
