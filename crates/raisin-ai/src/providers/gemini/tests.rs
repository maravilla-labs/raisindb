//! Tests for the Gemini provider.

use super::*;
use crate::provider::AIProviderTrait;

#[test]
fn test_provider_capabilities() {
    let provider = GeminiProvider::new("test-key");
    assert_eq!(provider.provider_name(), "gemini");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

#[test]
fn test_convert_messages() {
    let messages = vec![
        Message::user("Hello"),
        Message::assistant("Hi there!"),
        Message::user("What's the weather?"),
    ];

    let contents = GeminiProvider::convert_messages_to_contents(&messages);

    assert_eq!(contents.len(), 3);
    assert_eq!(contents[0].role, "user");
    assert_eq!(contents[1].role, "model");
    assert_eq!(contents[2].role, "user");
}

#[test]
fn test_extract_system_prompt() {
    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("Hello"),
    ];

    let system = GeminiProvider::extract_system_prompt(&messages, None);
    assert!(system.is_some());

    if let Some(content) = system {
        if let GeminiPart::Text { text } = &content.parts[0] {
            assert_eq!(text, "You are a helpful assistant.");
        } else {
            panic!("Expected text part");
        }
    }
}

#[test]
fn test_convert_model_info() {
    let provider = GeminiProvider::new("test-key");
    let gemini_model = GeminiModel {
        name: "models/gemini-1.5-pro".to_string(),
        display_name: "Gemini 1.5 Pro".to_string(),
        description: "A powerful model".to_string(),
        version: "001".to_string(),
        input_token_limit: Some(1048576),
        output_token_limit: Some(8192),
        supported_generation_methods: vec!["generateContent".to_string()],
    };

    let model_info = provider.convert_gemini_model(gemini_model);

    assert_eq!(model_info.id, "gemini-1.5-pro");
    assert_eq!(model_info.name, "Gemini 1.5 Pro");
    assert!(model_info.capabilities.chat);
    assert!(model_info.capabilities.tools);
    assert!(model_info.capabilities.vision);
}

// ── Streaming SSE parsing ─────────────────────────────────────────

use super::trait_impl::parse_gemini_sse_events;

#[test]
fn test_parse_sse_text_delta() {
    let sse = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hello"}]},"index":0}]}

"#;

    let chunks = parse_gemini_sse_events(sse, "gemini-1.5-flash");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "Hello");
}

#[test]
fn test_parse_sse_final_chunk_with_usage_metadata() {
    // Real Gemini wire format: the final chunk carries finishReason plus
    // cumulative usageMetadata.
    let sse = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":" world"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":7,"totalTokenCount":19},"modelVersion":"gemini-1.5-flash"}

"#;

    let chunks = parse_gemini_sse_events(sse, "gemini-1.5-flash");
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.delta, " world");
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    let usage = c.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 19);
}

#[test]
fn test_parse_sse_partial_usage_metadata_does_not_drop_text() {
    // Intermediate chunks can report usageMetadata WITHOUT
    // candidatesTokenCount. The text delta must survive parsing.
    let sse = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hi"}]},"index":0}],"usageMetadata":{"promptTokenCount":12,"totalTokenCount":12}}

"#;

    let chunks = parse_gemini_sse_events(sse, "gemini-1.5-flash");
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.delta, "Hi");
    let usage = c.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 0);
}

#[test]
fn test_parse_sse_usage_only_chunk_without_candidates() {
    // Trailing accounting chunk with no candidates must still emit usage.
    let sse = r#"data: {"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":9,"totalTokenCount":21}}

"#;

    let chunks = parse_gemini_sse_events(sse, "gemini-1.5-flash");
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert!(c.delta.is_empty());
    assert!(c.stop_reason.is_none());
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 21);
}

#[test]
fn test_parse_sse_finish_only_candidate_without_content() {
    // A final chunk whose candidate has finishReason but no content block.
    let sse = r#"data: {"candidates":[{"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":3,"totalTokenCount":8}}

"#;

    let chunks = parse_gemini_sse_events(sse, "gemini-1.5-flash");
    assert_eq!(chunks.len(), 1);

    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 8);
}
