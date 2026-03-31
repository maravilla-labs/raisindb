//! Tests for the model cache module.

use std::time::Duration;

use super::*;

#[test]
fn test_model_info_builder() {
    let model = ModelInfo::new("gpt-4", "GPT-4")
        .with_capabilities(ModelCapabilities::chat_with_tools())
        .with_context_window(8192)
        .with_max_output_tokens(4096);

    assert_eq!(model.id, "gpt-4");
    assert_eq!(model.name, "GPT-4");
    assert!(model.capabilities.chat);
    assert!(model.capabilities.tools);
    assert_eq!(model.context_window, Some(8192));
    assert_eq!(model.max_output_tokens, Some(4096));
}

#[test]
fn test_model_capabilities() {
    let chat = ModelCapabilities::chat();
    assert!(chat.chat);
    assert!(chat.streaming);
    assert!(!chat.tools);

    let chat_tools = ModelCapabilities::chat_with_tools();
    assert!(chat_tools.chat);
    assert!(chat_tools.tools);

    let vision = ModelCapabilities::vision();
    assert!(vision.chat);
    assert!(vision.vision);

    let embeddings = ModelCapabilities::embeddings();
    assert!(embeddings.embeddings);
    assert!(!embeddings.chat);
}

#[tokio::test]
async fn test_cache_get_put() {
    let cache = ModelCache::new();
    let models = vec![
        ModelInfo::new("model-1", "Model 1"),
        ModelInfo::new("model-2", "Model 2"),
    ];

    assert!(cache.get("test-provider").await.is_none());

    cache.put("test-provider", models.clone()).await;

    let cached = cache.get("test-provider").await;
    assert!(cached.is_some());
    let cached = cached.unwrap();
    assert_eq!(cached.len(), 2);
    assert_eq!(cached[0].id, "model-1");
}

#[tokio::test]
async fn test_cache_expiration() {
    let cache = ModelCache::with_ttl(Duration::from_millis(100));
    let models = vec![ModelInfo::new("model-1", "Model 1")];

    cache.put("test-provider", models).await;

    assert!(cache.get("test-provider").await.is_some());

    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(cache.get("test-provider").await.is_none());
}

#[tokio::test]
async fn test_cache_invalidate() {
    let cache = ModelCache::new();
    let models = vec![ModelInfo::new("model-1", "Model 1")];

    cache.put("test-provider", models).await;
    assert!(cache.get("test-provider").await.is_some());

    cache.invalidate("test-provider").await;
    assert!(cache.get("test-provider").await.is_none());
}

#[tokio::test]
async fn test_cache_clear() {
    let cache = ModelCache::new();

    cache
        .put("provider-1", vec![ModelInfo::new("m1", "M1")])
        .await;
    cache
        .put("provider-2", vec![ModelInfo::new("m2", "M2")])
        .await;

    cache.clear().await;

    assert!(cache.get("provider-1").await.is_none());
    assert!(cache.get("provider-2").await.is_none());
}

#[tokio::test]
async fn test_cache_cleanup() {
    let cache = ModelCache::with_ttl(Duration::from_millis(100));

    cache
        .put("provider-1", vec![ModelInfo::new("m1", "M1")])
        .await;

    cache
        .put_with_ttl(
            "provider-2",
            vec![ModelInfo::new("m2", "M2")],
            Duration::from_secs(3600),
        )
        .await;

    tokio::time::sleep(Duration::from_millis(150)).await;

    cache.cleanup().await;

    assert!(cache.get("provider-1").await.is_none());
    assert!(cache.get("provider-2").await.is_some());
}

#[test]
fn test_model_profile_builder() {
    let profile = ModelProfile::new("test-model", "Test Model", 100_000)
        .with_capabilities(ModelCapabilities::chat_with_tools())
        .with_max_output_tokens(4096)
        .with_costs(1.5, 7.5)
        .with_thinking_tags("<think>", "</think>")
        .with_native_json(true)
        .with_json_schema_transformer(SchemaTransformerType::Anthropic);

    assert_eq!(profile.id, "test-model");
    assert_eq!(profile.name, "Test Model");
    assert_eq!(profile.context_window, 100_000);
    assert_eq!(profile.max_output_tokens, Some(4096));
    assert_eq!(profile.cost_per_1k_input, Some(1.5));
    assert_eq!(profile.cost_per_1k_output, Some(7.5));
    assert_eq!(
        profile.thinking_tags,
        Some(("<think>".to_string(), "</think>".to_string()))
    );
    assert!(profile.supports_native_json);
    assert_eq!(
        profile.json_schema_transformer,
        Some(SchemaTransformerType::Anthropic)
    );
    assert!(profile.capabilities.chat);
    assert!(profile.capabilities.tools);
}

#[test]
fn test_model_profile_calculate_cost() {
    let profile = ModelProfile::new("test", "Test", 10_000).with_costs(3.0, 15.0);

    let cost = profile.calculate_cost(1000, 500);
    assert!(cost.is_some());
    assert_eq!(cost.unwrap(), 10.5);

    let cost = profile.calculate_cost(2500, 1000);
    assert!(cost.is_some());
    assert_eq!(cost.unwrap(), 22.5);
}

#[test]
fn test_model_profile_calculate_cost_no_pricing() {
    let profile = ModelProfile::new("test", "Test", 10_000);
    let cost = profile.calculate_cost(1000, 500);
    assert!(cost.is_none());
}

#[test]
fn test_model_profile_to_model_info() {
    let profile = ModelProfile::new("test-model", "Test Model", 100_000)
        .with_capabilities(ModelCapabilities::chat())
        .with_max_output_tokens(4096)
        .with_costs(1.5, 7.5);

    let model_info = profile.to_model_info();

    assert_eq!(model_info.id, "test-model");
    assert_eq!(model_info.name, "Test Model");
    assert_eq!(model_info.context_window, Some(100_000));
    assert_eq!(model_info.max_output_tokens, Some(4096));
    assert!(model_info.capabilities.chat);
    assert!(model_info.available);
}

#[test]
fn test_schema_transformer_type() {
    assert_eq!(SchemaTransformerType::OpenAI, SchemaTransformerType::OpenAI);
    assert_ne!(
        SchemaTransformerType::OpenAI,
        SchemaTransformerType::Anthropic
    );
}

#[test]
fn test_claude_sonnet_4_factory() {
    let profile = ModelProfile::claude_sonnet_4();

    assert_eq!(profile.id, "claude-sonnet-4-20250514");
    assert_eq!(profile.name, "Claude Sonnet 4");
    assert_eq!(profile.context_window, 200_000);
    assert_eq!(profile.max_output_tokens, Some(8192));
    assert_eq!(profile.cost_per_1k_input, Some(3.0));
    assert_eq!(profile.cost_per_1k_output, Some(15.0));
    assert!(profile.supports_native_json);
}

#[test]
fn test_gpt_4o_factory() {
    let profile = ModelProfile::gpt_4o();
    assert_eq!(profile.id, "gpt-4o");
    assert_eq!(profile.context_window, 128_000);
    assert!(profile.supports_native_json);
    assert!(profile.thinking_tags.is_none());
}

#[test]
fn test_llama_3_3_70b_factory() {
    let profile = ModelProfile::llama_3_3_70b();
    assert_eq!(profile.id, "llama-3.3-70b-versatile");
    assert!(!profile.capabilities.vision);
    assert!(profile.capabilities.tools);
}

#[test]
fn test_factory_cost_calculation() {
    let claude = ModelProfile::claude_sonnet_4();
    let gpt = ModelProfile::gpt_4o();
    let llama = ModelProfile::llama_3_3_70b();

    let tokens_in = 10_000;
    let tokens_out = 5_000;

    let claude_cost = claude.calculate_cost(tokens_in, tokens_out).unwrap();
    let gpt_cost = gpt.calculate_cost(tokens_in, tokens_out).unwrap();
    let llama_cost = llama.calculate_cost(tokens_in, tokens_out).unwrap();

    assert_eq!(claude_cost, 105.0);
    assert_eq!(gpt_cost, 75.0);
    assert_eq!(llama_cost, 9.85);
    assert!(llama_cost < gpt_cost);
    assert!(gpt_cost < claude_cost);
}

#[test]
fn test_serialization() {
    let profile = ModelProfile::claude_sonnet_4();
    let json = serde_json::to_string(&profile).unwrap();
    let deserialized: ModelProfile = serde_json::from_str(&json).unwrap();

    assert_eq!(profile.id, deserialized.id);
    assert_eq!(profile.name, deserialized.name);
    assert_eq!(profile.context_window, deserialized.context_window);
    assert_eq!(profile.cost_per_1k_input, deserialized.cost_per_1k_input);
    assert_eq!(profile.thinking_tags, deserialized.thinking_tags);
}
