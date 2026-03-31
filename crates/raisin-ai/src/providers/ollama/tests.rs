//! Tests for the Ollama provider.

use super::*;
use crate::provider::AIProviderTrait;
use std::collections::HashMap;

#[test]
fn test_default_base_url() {
    let provider = OllamaProvider::new();
    assert_eq!(provider.base_url, OLLAMA_DEFAULT_BASE);
}

#[test]
fn test_custom_base_url() {
    let provider = OllamaProvider::with_base_url("http://custom:1234/api");
    assert_eq!(provider.base_url, "http://custom:1234/api");
}

#[test]
fn test_provider_capabilities() {
    let provider = OllamaProvider::new();
    assert_eq!(provider.provider_name(), "ollama");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

#[test]
fn test_model_tool_support_detection() {
    // Tool-capable models (officially supported by Ollama)
    assert!(OllamaProvider::model_supports_tools_by_name("llama3.1"));
    assert!(OllamaProvider::model_supports_tools_by_name("llama3.1:8b"));
    assert!(OllamaProvider::model_supports_tools_by_name("llama3.1:70b"));
    assert!(OllamaProvider::model_supports_tools_by_name(
        "llama3-groq-tool-use"
    ));
    assert!(OllamaProvider::model_supports_tools_by_name(
        "llama3-groq-tool-use:8b"
    ));
    assert!(OllamaProvider::model_supports_tools_by_name("qwen3"));
    assert!(OllamaProvider::model_supports_tools_by_name("qwen3:8b"));
    assert!(OllamaProvider::model_supports_tools_by_name("qwen2.5"));
    assert!(OllamaProvider::model_supports_tools_by_name("qwen2.5:14b"));
    assert!(OllamaProvider::model_supports_tools_by_name("mistral"));
    assert!(OllamaProvider::model_supports_tools_by_name("mistral-nemo"));
    assert!(OllamaProvider::model_supports_tools_by_name("mixtral:8x7b"));
    assert!(OllamaProvider::model_supports_tools_by_name("gemma2:9b"));
    assert!(OllamaProvider::model_supports_tools_by_name("command-r"));
    assert!(OllamaProvider::model_supports_tools_by_name(
        "command-r-plus"
    ));
    assert!(OllamaProvider::model_supports_tools_by_name(
        "firefunction-v2"
    ));

    // Models without tool support (including llama3.2/3.3 due to Ollama template issues)
    assert!(!OllamaProvider::model_supports_tools_by_name("llama3.2"));
    assert!(!OllamaProvider::model_supports_tools_by_name("llama3.2:8b"));
    assert!(!OllamaProvider::model_supports_tools_by_name(
        "llama3.3:70b"
    ));
    assert!(!OllamaProvider::model_supports_tools_by_name("llama2"));
    assert!(!OllamaProvider::model_supports_tools_by_name("llama2:13b"));
    assert!(!OllamaProvider::model_supports_tools_by_name("phi3"));
    assert!(!OllamaProvider::model_supports_tools_by_name("codellama"));
    assert!(!OllamaProvider::model_supports_tools_by_name("llava"));
    assert!(!OllamaProvider::model_supports_tools_by_name(
        "unknown-model"
    ));
}

#[tokio::test]
async fn test_tool_support_caching() {
    let provider = OllamaProvider::new();

    // First check should populate cache
    let supports1 = provider.check_tool_support("qwen3:8b").await;
    assert!(supports1);

    // Second check should use cache
    let supports2 = provider.check_tool_support("qwen3:8b").await;
    assert_eq!(supports1, supports2);

    // Check cache contains the model
    let cache = provider.tool_support_cache.read().await;
    assert!(cache.contains_key("qwen3:8b"));
    assert_eq!(cache.get("qwen3:8b"), Some(&true));
}

#[tokio::test]
async fn test_tool_support_validation_with_unsupported_model() {
    use crate::types::{Message, Role, ToolDefinition};

    let provider = OllamaProvider::new();

    let request = crate::types::CompletionRequest {
        model: "llama2".to_string(), // Model that doesn't support tools
        messages: vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        tools: Some(vec![ToolDefinition::function(
            "test_function".to_string(),
            "A test function".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        )]),
        temperature: None,
        max_tokens: None,
        system: None,
        stream: false,
        response_format: None,
    };

    let result = provider.complete(request).await;
    assert!(result.is_err());

    if let Err(ProviderError::UnsupportedOperation(msg)) = result {
        assert!(msg.contains("llama2"));
        assert!(msg.contains("does not support tool calling"));
    } else {
        panic!("Expected UnsupportedOperation error");
    }
}

#[test]
fn test_embedding_model_detection() {
    // Test BERT architecture detection (all-minilm, nomic-embed-text)
    let bert_show = OllamaShowResponse {
        details: None,
        model_info: Some(HashMap::from([
            (
                "general.architecture".to_string(),
                serde_json::json!("bert"),
            ),
            ("bert.embedding_length".to_string(), serde_json::json!(384)),
            ("bert.pooling_type".to_string(), serde_json::json!(1)),
        ])),
    };
    assert!(OllamaProvider::is_embedding_model(&bert_show));

    // Test nomic-bert architecture
    let nomic_bert_show = OllamaShowResponse {
        details: None,
        model_info: Some(HashMap::from([
            (
                "general.architecture".to_string(),
                serde_json::json!("nomic-bert"),
            ),
            (
                "nomic-bert.embedding_length".to_string(),
                serde_json::json!(768),
            ),
            ("nomic-bert.pooling_type".to_string(), serde_json::json!(2)),
        ])),
    };
    assert!(OllamaProvider::is_embedding_model(&nomic_bert_show));

    // Test XLM-RoBERTa architecture (multilingual embeddings)
    let xlm_show = OllamaShowResponse {
        details: None,
        model_info: Some(HashMap::from([(
            "general.architecture".to_string(),
            serde_json::json!("xlm-roberta"),
        )])),
    };
    assert!(OllamaProvider::is_embedding_model(&xlm_show));

    // Test model with only pooling_type (should detect as embedding)
    let pooling_only = OllamaShowResponse {
        details: None,
        model_info: Some(HashMap::from([
            (
                "general.architecture".to_string(),
                serde_json::json!("unknown"),
            ),
            ("unknown.pooling_type".to_string(), serde_json::json!(1)),
        ])),
    };
    assert!(OllamaProvider::is_embedding_model(&pooling_only));

    // Test LLaMA architecture (NOT an embedding model)
    let llama_show = OllamaShowResponse {
        details: None,
        model_info: Some(HashMap::from([(
            "general.architecture".to_string(),
            serde_json::json!("llama"),
        )])),
    };
    assert!(!OllamaProvider::is_embedding_model(&llama_show));

    // Test mistral architecture (NOT an embedding model)
    let mistral_show = OllamaShowResponse {
        details: None,
        model_info: Some(HashMap::from([(
            "general.architecture".to_string(),
            serde_json::json!("mistral"),
        )])),
    };
    assert!(!OllamaProvider::is_embedding_model(&mistral_show));

    // Test empty model_info
    let empty_show = OllamaShowResponse {
        details: None,
        model_info: None,
    };
    assert!(!OllamaProvider::is_embedding_model(&empty_show));
}
