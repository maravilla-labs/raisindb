//! Tests for the Azure OpenAI provider.

use super::*;
use crate::provider::AIProviderTrait;

#[test]
fn test_provider_capabilities() {
    let provider = AzureOpenAIProvider::new("test-key", "https://test.openai.azure.com");
    assert_eq!(provider.provider_name(), "azure_openai");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

#[test]
fn test_endpoint_normalization() {
    let provider = AzureOpenAIProvider::new("test-key", "https://test.openai.azure.com/");
    assert_eq!(provider.endpoint, "https://test.openai.azure.com");
}

#[test]
fn test_build_model_info() {
    let provider = AzureOpenAIProvider::new("test-key", "https://test.openai.azure.com");

    let gpt4o = provider.build_model_info("gpt-4o");
    assert!(gpt4o.capabilities.tools);
    assert!(gpt4o.capabilities.vision);
    assert_eq!(gpt4o.context_window, Some(128000));

    let gpt35 = provider.build_model_info("gpt-35-turbo");
    assert!(gpt35.capabilities.tools);
    assert!(!gpt35.capabilities.vision);
    assert_eq!(gpt35.context_window, Some(4096));
}

#[test]
fn test_convert_messages() {
    let msg = Message::user("Hello");
    let azure_msg = AzureOpenAIProvider::convert_message(&msg);

    if let AzureChatMessage::User { content } = azure_msg {
        assert_eq!(content, "Hello");
    } else {
        panic!("Expected User message");
    }
}

// ── Streaming SSE parsing ─────────────────────────────────────────

use super::trait_impl::parse_azure_sse_events;

#[test]
fn test_parse_sse_text_delta() {
    let sse = r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}],"model":"gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_azure_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "Hello");
}

#[test]
fn test_parse_sse_usage_on_trailing_empty_choices_chunk() {
    // With stream_options.include_usage, Azure sends the finish chunk with
    // usage:null and then a TRAILING chunk with empty choices carrying usage.
    let sse = r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":null,"model":"gpt-4o"}

data: {"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":25,"completion_tokens":8,"total_tokens":33},"model":"gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_azure_sse_events(sse);
    assert_eq!(chunks.len(), 2);

    let finish = chunks[0].as_ref().unwrap();
    assert_eq!(finish.stop_reason.as_deref(), Some("stop"));
    assert!(finish.usage.is_none());

    let trailing = chunks[1].as_ref().unwrap();
    assert!(trailing.stop_reason.is_none());
    let usage = trailing.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 25);
    assert_eq!(usage.completion_tokens, 8);
    assert_eq!(usage.total_tokens, 33);
}

#[test]
fn test_parse_sse_usage_on_finish_chunk_not_duplicated() {
    // Some gateway deployments attach usage directly to the finish chunk;
    // it must be captured exactly once.
    let sse = r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"model":"gpt-4o"}

data: [DONE]
"#;

    let chunks = parse_azure_sse_events(sse);
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 15);
}
