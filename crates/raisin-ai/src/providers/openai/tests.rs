//! Tests for the OpenAI provider.

use super::*;
use crate::provider::AIProviderTrait;

#[test]
fn test_validate_chat_model() {
    assert!(OpenAIProvider::validate_chat_model("gpt-4o").is_ok());
    assert!(OpenAIProvider::validate_chat_model("gpt-4-turbo").is_ok());
    assert!(OpenAIProvider::validate_chat_model("o1").is_ok());
    assert!(OpenAIProvider::validate_chat_model("invalid-model").is_err());
}

#[test]
fn test_provider_capabilities() {
    let provider = OpenAIProvider::new("test-key");
    assert_eq!(provider.provider_name(), "openai");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

// ── Streaming SSE parsing (Responses API) ─────────────────────────

use super::trait_impl::parse_sse_events;

#[test]
fn test_parse_sse_text_delta() {
    let sse = r#"data: {"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello"}

"#;

    let chunks = parse_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "Hello");
}

#[test]
fn test_parse_sse_completed_carries_usage() {
    let sse = r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","model":"gpt-4o","output":[],"usage":{"input_tokens":50,"output_tokens":20,"total_tokens":70}}}

"#;

    let chunks = parse_sse_events(sse);
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    assert_eq!(c.model.as_deref(), Some("gpt-4o"));
    let usage = c.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 50);
    assert_eq!(usage.completion_tokens, 20);
    assert_eq!(usage.total_tokens, 70);
}

#[test]
fn test_parse_sse_incomplete_carries_usage() {
    // Terminal event when generation stops early (e.g. max_output_tokens):
    // final usage must still be captured.
    let sse = r#"data: {"type":"response.incomplete","response":{"id":"resp_1","status":"incomplete","model":"gpt-4o","output":[],"usage":{"input_tokens":50,"output_tokens":128,"total_tokens":178}}}

"#;

    let chunks = parse_sse_events(sse);
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("length"));
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 178);
}
