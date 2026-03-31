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
}
