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
