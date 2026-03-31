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
