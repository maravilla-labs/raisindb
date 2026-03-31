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
