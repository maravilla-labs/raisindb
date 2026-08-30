//! Common types for AI requests and responses.
//!
//! This module defines the unified request and response types used across
//! different AI providers. These types provide a consistent interface regardless
//! of the underlying provider (OpenAI, Anthropic, etc.).
//!
//! ## Multimodal Support
//!
//! Messages can contain both text and image content via the [`MessageContent`] type:
//!
//! ```rust
//! use raisin_ai::types::{Message, MessageContent, ContentPart, Role};
//!
//! // Text-only message (most common)
//! let text_msg = Message::user("Hello!");
//!
//! // Multimodal message with text and image
//! let multimodal_msg = Message {
//!     role: Role::User,
//!     content_parts: Some(MessageContent::Parts(vec![
//!         ContentPart::Text { text: "What's in this image?".to_string() },
//!         ContentPart::Image {
//!             data: "base64-encoded-image-data".to_string(),
//!             media_type: "image/jpeg".to_string(),
//!         },
//!     ])),
//!     ..Default::default()
//! };
//! ```

mod content;
pub mod message;
mod request;
mod response;
mod tools;

// Re-export all public types so that `use raisin_ai::types::X` keeps working.
pub use content::{ContentPart, MessageContent};
pub use message::{Message, Role};
pub use request::{CompletionRequest, JsonSchemaSpec, ResponseFormat};
pub use response::{CompletionResponse, StreamChunk, Usage};
pub use tools::{FunctionCall, FunctionDefinition, ToolCall, ToolDefinition};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_constructors() {
        let user_msg = Message::user("Hello");
        assert_eq!(user_msg.role, Role::User);
        assert_eq!(user_msg.content, "Hello");

        let assistant_msg = Message::assistant("Hi there");
        assert_eq!(assistant_msg.role, Role::Assistant);
        assert_eq!(assistant_msg.content, "Hi there");

        let system_msg = Message::system("You are helpful");
        assert_eq!(system_msg.role, Role::System);
        assert_eq!(system_msg.content, "You are helpful");
    }

    #[test]
    fn test_completion_request_builder() {
        let request = CompletionRequest::new("gpt-4".to_string(), vec![Message::user("Hello")])
            .with_temperature(0.8)
            .with_max_tokens(100)
            .with_streaming();

        assert_eq!(request.model, "gpt-4");
        assert_eq!(request.temperature, Some(0.8));
        assert_eq!(request.max_tokens, Some(100));
        assert!(request.stream);
    }

    #[test]
    fn test_tool_definition() {
        use serde_json::json;

        let tool = ToolDefinition::function(
            "test_fn".to_string(),
            "A test function".to_string(),
            json!({"type": "object"}),
        );

        assert_eq!(tool.tool_type, "function");
        assert_eq!(tool.function.name, "test_fn");
        assert_eq!(tool.function.description, "A test function");
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("Test message");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();

        assert_eq!(msg.role, deserialized.role);
        assert_eq!(msg.content, deserialized.content);
    }

    #[test]
    fn test_message_deserialize_string_content() {
        // Standard text-only message (OpenAI format)
        let json = r#"{"role": "user", "content": "Hello, world!"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();

        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello, world!");
        assert!(msg.content_parts.is_none());
    }

    #[test]
    fn test_message_deserialize_array_content_multimodal() {
        // OpenAI-style multimodal message with content as array
        let json = r#"{
            "role": "user",
            "content": [
                {"type": "text", "text": "What's in this image?"},
                {"type": "image", "data": "base64data", "media_type": "image/jpeg"}
            ]
        }"#;
        let msg: Message = serde_json::from_str(json).unwrap();

        assert_eq!(msg.role, Role::User);
        // content should be extracted from text parts
        assert_eq!(msg.content, "What's in this image?");
        // content_parts should be set
        assert!(msg.content_parts.is_some());
        assert!(msg.has_images());

        // Verify the parts
        if let Some(MessageContent::Parts(parts)) = &msg.content_parts {
            assert_eq!(parts.len(), 2);
            assert!(
                matches!(&parts[0], ContentPart::Text { text } if text == "What's in this image?")
            );
            assert!(matches!(&parts[1], ContentPart::Image { data, media_type }
                if data == "base64data" && media_type == "image/jpeg"));
        } else {
            panic!("Expected Parts content");
        }
    }

    #[test]
    fn test_message_deserialize_null_content() {
        // Content can be null (e.g., for tool messages)
        let json = r#"{"role": "assistant", "content": null}"#;
        let msg: Message = serde_json::from_str(json).unwrap();

        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "");
    }

    #[test]
    fn test_message_deserialize_with_explicit_content_parts() {
        // Also support explicit content_parts field (untagged enum format)
        // MessageContent::Parts deserializes as array, MessageContent::Text as string
        let json = r#"{
            "role": "user",
            "content": "fallback text",
            "content_parts": [
                {"type": "text", "text": "Primary text"}
            ]
        }"#;
        let msg: Message = serde_json::from_str(json).unwrap();

        assert_eq!(msg.role, Role::User);
        // When both are present, content string is used
        assert_eq!(msg.content, "fallback text");
        // But content_parts is also available
        assert!(msg.content_parts.is_some());
    }

    #[test]
    fn test_completion_request_with_multimodal_message() {
        // Full integration test: CompletionRequest with multimodal message
        let json = r#"{
            "model": "gpt-4-vision",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Describe this"},
                    {"type": "image", "data": "abc123", "media_type": "image/png"}
                ]}
            ]
        }"#;
        let request: CompletionRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.model, "gpt-4-vision");
        assert_eq!(request.messages.len(), 2);

        // First message is text-only
        assert!(!request.messages[0].is_multimodal());
        assert_eq!(request.messages[0].content, "Hello");

        // Second message is multimodal
        assert!(request.messages[1].is_multimodal());
        assert!(request.messages[1].has_images());
        assert_eq!(request.messages[1].content, "Describe this");
    }

    /// Every spelling of an image content part a caller might plausibly write
    /// must reach `ContentPart::Image` or `ContentPart::ImageUrl`.
    ///
    /// Before this, exactly ONE of these parsed — `{type: "image_url", url}` —
    /// and it is the only one no real client emits. The OpenAI form was a
    /// deserialization error surfacing from inside `CompletionRequest`, and the
    /// `data:` URL forms parsed into `ImageUrl`, which `first_image()` returns
    /// `None` for: so the image was dropped with no error and Ollama sent a
    /// text-only request. "Multimodal is supported" was true of the types and
    /// false of everything that could reach them.
    #[test]
    fn every_image_wire_spelling_deserializes() {
        use crate::types::content::ContentPart;

        // A one-pixel PNG is irrelevant here; only the split matters.
        const B64: &str = "iVBORw0KGgo=";

        let cases: Vec<(&str, String)> = vec![
            (
                "native flat",
                format!(r#"{{"type":"image","data":"{B64}","media_type":"image/png"}}"#),
            ),
            (
                "anthropic source",
                format!(
                    r#"{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{B64}"}}}}"#
                ),
            ),
            (
                "openai chat image_url object with a data: url",
                format!(
                    r#"{{"type":"image_url","image_url":{{"url":"data:image/png;base64,{B64}"}}}}"#
                ),
            ),
            (
                "openai chat image_url as a bare string",
                format!(r#"{{"type":"image_url","image_url":"data:image/png;base64,{B64}"}}"#),
            ),
            (
                "the legacy raisin form",
                format!(r#"{{"type":"image_url","url":"data:image/png;base64,{B64}"}}"#),
            ),
            (
                "openai responses input_image",
                format!(r#"{{"type":"input_image","image_url":"data:image/png;base64,{B64}"}}"#),
            ),
        ];

        for (label, json) in cases {
            let part: ContentPart = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("`{label}` must deserialize: {e}\n{json}"));
            let (data, media_type) = part.as_image_data().unwrap_or_else(|| {
                panic!("`{label}` must normalize to inline base64, got {part:?}")
            });
            assert_eq!(data, B64, "{label}: base64 payload must survive intact");
            assert_eq!(media_type, "image/png", "{label}: media type must survive");
        }
    }

    /// A REMOTE url stays a url. It must not be mangled into base64, and the
    /// `data:`-splitting must not fire on it.
    #[test]
    fn a_remote_image_url_stays_a_url() {
        use crate::types::content::ContentPart;

        let part: ContentPart = serde_json::from_str(
            r#"{"type":"image_url","image_url":{"url":"https://example.test/cat.jpg"}}"#,
        )
        .unwrap();
        assert_eq!(part.as_image_url(), Some("https://example.test/cat.jpg"));
        assert!(part.as_image_data().is_none());
    }

    /// A `data:` URL that is NOT base64 must not be silently treated as one.
    ///
    /// Splitting it anyway would hand a provider a percent-encoded payload
    /// labelled as base64 — a 400 from the vendor whose message names the field
    /// and not the cause. Carrying it through as a URL fails at the provider
    /// with something a caller can act on.
    #[test]
    fn a_non_base64_data_url_is_not_split() {
        use crate::types::content::ContentPart;

        let part = ContentPart::from_url("data:image/svg+xml,%3Csvg%2F%3E");
        assert!(part.as_image_data().is_none());
        assert!(part.as_image_url().is_some());
    }

    /// The text spellings, including the Responses API's `input_text`.
    #[test]
    fn both_text_spellings_deserialize() {
        use crate::types::content::ContentPart;

        for json in [
            r#"{"type":"text","text":"hi"}"#,
            r#"{"type":"input_text","text":"hi"}"#,
        ] {
            let part: ContentPart = serde_json::from_str(json).unwrap();
            assert_eq!(part.as_text(), Some("hi"), "{json}");
        }
    }

    /// The end-to-end shape a Studio captioning trigger will send: the OpenAI
    /// `messages` array, straight off `raisin.ai.completion`'s request JSON.
    ///
    /// This is the exact payload documented for the binding, so it is asserted
    /// rather than described.
    #[test]
    fn the_documented_js_call_signature_parses_and_keeps_its_image() {
        let json = r#"{
            "model": "gemma4:latest",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Describe this image."},
                    {"type": "image_url", "image_url": {
                        "url": "data:image/png;base64,iVBORw0KGgo="
                    }}
                ]}
            ]
        }"#;
        let request: CompletionRequest = serde_json::from_str(json).unwrap();

        let msg = &request.messages[0];
        assert!(msg.has_images(), "the image part must survive parsing");
        assert_eq!(
            msg.image_parts().len(),
            1,
            "exactly one image, and it must be reachable as an image part"
        );
        assert_eq!(
            msg.first_image(),
            Some(("iVBORw0KGgo=", "image/png")),
            "the provider layer reads it through `first_image`, so THAT is what \
             has to be populated — an `ImageUrl` here reads as 'no image'"
        );
        assert_eq!(
            msg.effective_text(),
            "Describe this image.",
            "the prompt text must still be the prompt text"
        );
    }

    /// Two images in one message must both be carried.
    #[test]
    fn a_second_image_is_not_dropped() {
        let json = r#"{
            "model": "m",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Compare these."},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
                    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,BBBB"}}
                ]}
            ]
        }"#;
        let request: CompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.messages[0].image_parts().len(), 2);
    }

    /// A provider that cannot carry images must refuse, not ignore.
    #[test]
    fn the_shared_guard_refuses_a_dropped_image() {
        use crate::provider::reject_unsupported_images;

        let text_only = vec![Message::user("hello")];
        assert!(reject_unsupported_images("groq", &text_only).is_ok());

        let with_image = vec![Message::user_multimodal(vec![
            ContentPart::text("what is this"),
            ContentPart::image("AAAA", "image/png"),
        ])];
        let err = reject_unsupported_images("groq", &with_image)
            .expect_err("an image through a provider that drops it must be an error");
        let msg = err.to_string();
        assert!(
            msg.contains("groq"),
            "the error must name the provider: {msg}"
        );
        assert!(
            msg.contains("ollama"),
            "and must name a provider that DOES work, or the caller is stuck: {msg}"
        );
    }
}
