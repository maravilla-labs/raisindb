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

/// The Responses-API body an image actually goes out in.
///
/// There is no live OpenAI leg in this suite, so the wire SHAPE is what gets
/// asserted — and the shape is where this API is easy to get wrong in a way
/// that only shows up as a 400 naming a field and not a reason:
///
///   * the Responses API says `input_text` / `input_image`, NOT the Chat
///     Completions API's `text` / `image_url`;
///   * `input_image`'s `image_url` is a BARE STRING, not `{ "url": … }`;
///   * a `data:` URL is a legal value there, which is what lets an inline
///     base64 part go out with no extra round trip.
#[test]
fn a_multimodal_user_message_serializes_as_responses_api_parts() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": "What is in this image?" },
                { "type": "image_url", "image_url": {
                    "url": "data:image/png;base64,AAAB"
                }}
            ]
        }]
    });
    let request: crate::types::CompletionRequest = serde_json::from_value(json).unwrap();
    let body = OpenAIProvider::build_responses_request(&request, false);
    let wire = serde_json::to_value(&body).unwrap();

    let content = &wire["input"][0]["content"];
    assert!(
        content.is_array(),
        "a message carrying an image must use the ARRAY content form: {wire}"
    );
    assert_eq!(content[0]["type"], "input_text");
    assert_eq!(content[0]["text"], "What is in this image?");
    assert_eq!(content[1]["type"], "input_image");
    assert_eq!(
        content[1]["image_url"], "data:image/png;base64,AAAB",
        "`input_image.image_url` is a bare string on the Responses API, not an object"
    );
}

/// A TEXT-ONLY message must keep the bare-string form byte for byte.
///
/// The array form is a different request body. Switching every existing
/// text-only call onto a newly-written code path in order to support images
/// would put the risk on the 99% of requests that gain nothing from it.
#[test]
fn a_text_only_message_keeps_the_bare_string_content_form() {
    let request = crate::types::CompletionRequest::new(
        "gpt-4o".to_string(),
        vec![crate::types::Message::user("hello")],
    );
    let wire =
        serde_json::to_value(OpenAIProvider::build_responses_request(&request, false)).unwrap();
    assert_eq!(
        wire["input"][0]["content"], "hello",
        "text-only content must stay a plain string: {wire}"
    );
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
