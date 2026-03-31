//! Tests for the configuration module.

use super::*;

#[test]
fn test_tenant_config_new() {
    let config = TenantAIConfig::new("test-tenant".to_string());
    assert_eq!(config.tenant_id, "test-tenant");
    assert!(config.providers.is_empty());
}

#[test]
fn test_provider_config_new() {
    let config = AIProviderConfig::new(AIProvider::OpenAI);
    assert_eq!(config.provider, AIProvider::OpenAI);
    assert!(config.enabled);
    assert!(config.api_key_encrypted.is_none());
    assert!(config.models.is_empty());
}

#[test]
fn test_model_config_new() {
    let config = AIModelConfig::new("gpt-4".to_string(), "GPT-4".to_string());
    assert_eq!(config.model_id, "gpt-4");
    assert_eq!(config.display_name, "GPT-4");
    assert_eq!(config.default_temperature, 0.7);
    assert_eq!(config.default_max_tokens, 1024);
    assert!(!config.is_default);
}

#[test]
fn test_provider_default_endpoints() {
    assert_eq!(
        AIProvider::OpenAI.default_endpoint(),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(
        AIProvider::Anthropic.default_endpoint(),
        Some("https://api.anthropic.com/v1")
    );
    assert_eq!(
        AIProvider::Ollama.default_endpoint(),
        Some("http://localhost:11434")
    );
    assert_eq!(AIProvider::AzureOpenAI.default_endpoint(), None);
    assert_eq!(AIProvider::Custom.default_endpoint(), None);
}

#[test]
fn test_provider_requires_api_key() {
    assert!(AIProvider::OpenAI.requires_api_key());
    assert!(AIProvider::Anthropic.requires_api_key());
    assert!(AIProvider::Google.requires_api_key());
    assert!(AIProvider::AzureOpenAI.requires_api_key());
    assert!(!AIProvider::Ollama.requires_api_key());
    assert!(!AIProvider::Custom.requires_api_key());
}

#[test]
fn test_get_model() {
    let mut config = TenantAIConfig::new("test-tenant".to_string());
    let mut provider = AIProviderConfig::new(AIProvider::OpenAI);
    provider
        .models
        .push(AIModelConfig::new("gpt-4".to_string(), "GPT-4".to_string()));
    config.providers.push(provider);

    let result = config.get_model("gpt-4");
    assert!(result.is_some());
    let (_, model) = result.unwrap();
    assert_eq!(model.model_id, "gpt-4");

    assert!(config.get_model("nonexistent").is_none());
}

#[test]
fn test_new_providers_endpoints() {
    assert_eq!(
        AIProvider::Groq.default_endpoint(),
        Some("https://api.groq.com/openai/v1")
    );
    assert_eq!(
        AIProvider::OpenRouter.default_endpoint(),
        Some("https://openrouter.ai/api/v1")
    );
    assert_eq!(AIProvider::Bedrock.default_endpoint(), None);
}

#[test]
fn test_new_providers_require_api_key() {
    assert!(AIProvider::Groq.requires_api_key());
    assert!(AIProvider::OpenRouter.requires_api_key());
    assert!(AIProvider::Bedrock.requires_api_key());
}

#[test]
fn test_new_providers_serialization() {
    let groq = AIProvider::Groq;
    let json = serde_json::to_string(&groq).unwrap();
    assert_eq!(json, "\"groq\"");
    let deserialized: AIProvider = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, AIProvider::Groq);

    let openrouter = AIProvider::OpenRouter;
    let json = serde_json::to_string(&openrouter).unwrap();
    assert_eq!(json, "\"openrouter\"");
    let deserialized: AIProvider = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, AIProvider::OpenRouter);

    let bedrock = AIProvider::Bedrock;
    let json = serde_json::to_string(&bedrock).unwrap();
    assert_eq!(json, "\"bedrock\"");
    let deserialized: AIProvider = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, AIProvider::Bedrock);
}

#[test]
fn test_local_provider() {
    let local = AIProvider::Local;
    let json = serde_json::to_string(&local).unwrap();
    assert_eq!(json, "\"local\"");
    let deserialized: AIProvider = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, AIProvider::Local);

    assert_eq!(AIProvider::Local.default_endpoint(), None);
    assert!(!AIProvider::Local.requires_api_key());
    assert!(AIProvider::Local.is_local());
    assert!(!AIProvider::OpenAI.is_local());
}

#[test]
fn test_serde_name() {
    assert_eq!(AIProvider::OpenAI.serde_name(), "openai");
    assert_eq!(AIProvider::Anthropic.serde_name(), "anthropic");
    assert_eq!(AIProvider::Google.serde_name(), "google");
    assert_eq!(AIProvider::Ollama.serde_name(), "ollama");
    assert_eq!(AIProvider::AzureOpenAI.serde_name(), "azure_openai");
    assert_eq!(AIProvider::Groq.serde_name(), "groq");
    assert_eq!(AIProvider::OpenRouter.serde_name(), "openrouter");
    assert_eq!(AIProvider::Bedrock.serde_name(), "bedrock");
    assert_eq!(AIProvider::Custom.serde_name(), "custom");
    assert_eq!(AIProvider::Local.serde_name(), "local");
}

#[test]
fn test_from_serde_name() {
    assert_eq!(
        AIProvider::from_serde_name("openai"),
        Some(AIProvider::OpenAI)
    );
    assert_eq!(
        AIProvider::from_serde_name("anthropic"),
        Some(AIProvider::Anthropic)
    );
    assert_eq!(
        AIProvider::from_serde_name("local"),
        Some(AIProvider::Local)
    );
    assert_eq!(AIProvider::from_serde_name("unknown"), None);
    assert_eq!(AIProvider::from_serde_name(""), None);
}

#[test]
fn test_get_model_with_prefix() {
    let mut config = TenantAIConfig::new("test-tenant".to_string());

    let mut openai = AIProviderConfig::new(AIProvider::OpenAI);
    openai.models.push(AIModelConfig::new(
        "gpt-4o".to_string(),
        "GPT-4o".to_string(),
    ));
    config.providers.push(openai);

    let mut local = AIProviderConfig::new(AIProvider::Local);
    local.models.push(AIModelConfig::new(
        "moondream".to_string(),
        "Moondream".to_string(),
    ));
    local
        .models
        .push(AIModelConfig::new("clip".to_string(), "CLIP".to_string()));
    config.providers.push(local);

    let result = config.get_model("openai:gpt-4o");
    assert!(result.is_some());
    let (provider, model) = result.unwrap();
    assert_eq!(provider.provider, AIProvider::OpenAI);
    assert_eq!(model.model_id, "gpt-4o");

    let result = config.get_model("local:moondream");
    assert!(result.is_some());
    let (provider, model) = result.unwrap();
    assert_eq!(provider.provider, AIProvider::Local);
    assert_eq!(model.model_id, "moondream");

    let result = config.get_model("clip");
    assert!(result.is_some());
    let (provider, model) = result.unwrap();
    assert_eq!(provider.provider, AIProvider::Local);
    assert_eq!(model.model_id, "clip");

    assert!(config.get_model("anthropic:gpt-4o").is_none());
    assert!(config.get_model("unknown:model").is_none());
    assert!(config.get_model("openai:moondream").is_none());
}

#[test]
fn test_parse_model_id() {
    let (provider, model) = TenantAIConfig::parse_model_id("openai:gpt-4o");
    assert_eq!(provider, Some("openai"));
    assert_eq!(model, "gpt-4o");

    let (provider, model) = TenantAIConfig::parse_model_id("local:moondream");
    assert_eq!(provider, Some("local"));
    assert_eq!(model, "moondream");

    let (provider, model) = TenantAIConfig::parse_model_id("gpt-4o");
    assert_eq!(provider, None);
    assert_eq!(model, "gpt-4o");

    let (provider, model) = TenantAIConfig::parse_model_id("unknown:model");
    assert_eq!(provider, None);
    assert_eq!(model, "unknown:model");

    let (provider, model) = TenantAIConfig::parse_model_id("text-embedding-3:small");
    assert_eq!(provider, None);
    assert_eq!(model, "text-embedding-3:small");
}
