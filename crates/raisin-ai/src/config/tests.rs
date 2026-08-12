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
    assert_eq!(config.kind, AIProvider::OpenAI);
    assert_eq!(config.slug, "openai");
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
    assert_eq!(provider.kind, AIProvider::OpenAI);
    assert_eq!(model.model_id, "gpt-4o");

    let result = config.get_model("local:moondream");
    assert!(result.is_some());
    let (provider, model) = result.unwrap();
    assert_eq!(provider.kind, AIProvider::Local);
    assert_eq!(model.model_id, "moondream");

    let result = config.get_model("clip");
    assert!(result.is_some());
    let (provider, model) = result.unwrap();
    assert_eq!(provider.kind, AIProvider::Local);
    assert_eq!(model.model_id, "clip");

    assert!(config.get_model("anthropic:gpt-4o").is_none());
    assert!(config.get_model("unknown:model").is_none());
    assert!(config.get_model("openai:moondream").is_none());
}

#[test]
fn test_parse_model_id() {
    let mut config = TenantAIConfig::new("test-tenant".to_string());
    config
        .providers
        .push(AIProviderConfig::new(AIProvider::OpenAI));
    config
        .providers
        .push(AIProviderConfig::new(AIProvider::Local));

    let (provider, model) = config.parse_model_id("openai:gpt-4o");
    assert_eq!(provider, Some("openai"));
    assert_eq!(model, "gpt-4o");

    let (provider, model) = config.parse_model_id("local:moondream");
    assert_eq!(provider, Some("local"));
    assert_eq!(model, "moondream");

    let (provider, model) = config.parse_model_id("gpt-4o");
    assert_eq!(provider, None);
    assert_eq!(model, "gpt-4o");

    let (provider, model) = config.parse_model_id("unknown:model");
    assert_eq!(provider, None);
    assert_eq!(model, "unknown:model");

    let (provider, model) = config.parse_model_id("text-embedding-3:small");
    assert_eq!(provider, None);
    assert_eq!(model, "text-embedding-3:small");
}

#[test]
fn test_prefix_is_a_slug_not_a_kind_so_an_unconfigured_kind_is_not_a_prefix() {
    // `anthropic` is a perfectly good provider *kind*, but this tenant has not
    // configured an entry with that slug, so it is not a valid model prefix
    // here. Under the old static-table check it would have been.
    let mut config = TenantAIConfig::new("test-tenant".to_string());
    config.providers.push(AIProviderConfig::with_slug(
        "claude-eu",
        AIProvider::Anthropic,
    ));

    let (provider, model) = config.parse_model_id("anthropic:claude-sonnet-4");
    assert_eq!(provider, None);
    assert_eq!(model, "anthropic:claude-sonnet-4");

    let (provider, model) = config.parse_model_id("claude-eu:claude-sonnet-4");
    assert_eq!(provider, Some("claude-eu"));
    assert_eq!(model, "claude-sonnet-4");
}

#[test]
fn test_stored_entry_without_a_slug_deserializes_with_the_kind_serde_name() {
    // Simulates a config written before slugs existed. RocksDB persists these
    // with rmp_serde::to_vec_named, so round-trip through that exact encoder:
    // a JSON-only test would not prove the MessagePack path defaults too.
    #[derive(serde::Serialize)]
    struct LegacyProvider {
        provider: AIProvider,
        enabled: bool,
        models: Vec<AIModelConfig>,
    }
    #[derive(serde::Serialize)]
    struct LegacyConfig {
        tenant_id: String,
        providers: Vec<LegacyProvider>,
    }

    let legacy = LegacyConfig {
        tenant_id: "test-tenant".to_string(),
        providers: vec![
            LegacyProvider {
                provider: AIProvider::OpenAI,
                enabled: true,
                models: vec![AIModelConfig::new(
                    "gpt-4o".to_string(),
                    "GPT-4o".to_string(),
                )],
            },
            LegacyProvider {
                provider: AIProvider::Custom,
                enabled: true,
                models: vec![AIModelConfig::new(
                    "maravilla/smart".to_string(),
                    "Maravilla Smart".to_string(),
                )],
            },
        ],
    };

    let bytes = rmp_serde::to_vec_named(&legacy).unwrap();
    let config: TenantAIConfig = rmp_serde::from_slice(&bytes).unwrap();

    assert_eq!(config.providers[0].slug, "openai");
    assert_eq!(config.providers[1].slug, "custom");

    // The whole point of the default: model ids stored against the old scheme
    // must keep resolving without a data migration.
    let (provider, model) = config.get_model("openai:gpt-4o").unwrap();
    assert_eq!(provider.kind, AIProvider::OpenAI);
    assert_eq!(model.model_id, "gpt-4o");

    let (provider, model) = config.get_model("custom:maravilla/smart").unwrap();
    assert_eq!(provider.kind, AIProvider::Custom);
    assert_eq!(model.model_id, "maravilla/smart");
}

#[test]
fn test_legacy_slug_default_applies_to_json_bodies_too() {
    // The shim lives on the type, not in the RocksDB repository, so an HTTP
    // body missing `slug` gets the same treatment.
    let entry: AIProviderConfig =
        serde_json::from_str(r#"{"provider":"ollama","enabled":true,"models":[]}"#).unwrap();
    assert_eq!(entry.slug, "ollama");
    assert_eq!(entry.kind, AIProvider::Ollama);
}

#[test]
fn test_the_wire_name_of_kind_stays_provider() {
    // Renaming the wire key would make every stored provider type read as
    // missing, i.e. silently drop the entry.
    let entry = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["provider"], "custom");
    assert_eq!(json["slug"], "marvel");
    assert!(json.get("kind").is_none());
}

#[test]
fn test_two_entries_of_the_same_kind_resolve_independently_by_slug() {
    // The entire point of the change: two OpenAI-compatible endpoints, one
    // tenant. Under the enum-keyed scheme the second was unreachable.
    let mut config = TenantAIConfig::new("test-tenant".to_string());

    let mut marvel = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
    marvel.api_endpoint = Some("https://marvel.maravilla.cloud/v1".to_string());
    marvel.display_name = Some("Maravilla".to_string());
    marvel
        .models
        .push(AIModelConfig::new("smart".to_string(), "Smart".to_string()));
    config.providers.push(marvel);

    let mut vllm = AIProviderConfig::with_slug("my-vllm", AIProvider::Custom);
    vllm.api_endpoint = Some("http://10.0.0.4:8000/v1".to_string());
    vllm.models
        .push(AIModelConfig::new("smart".to_string(), "Smart".to_string()));
    config.providers.push(vllm);

    let (provider, model) = config.get_model("marvel:smart").unwrap();
    assert_eq!(
        provider.api_endpoint.as_deref(),
        Some("https://marvel.maravilla.cloud/v1")
    );
    assert_eq!(model.model_id, "smart");

    let (provider, _) = config.get_model("my-vllm:smart").unwrap();
    assert_eq!(
        provider.api_endpoint.as_deref(),
        Some("http://10.0.0.4:8000/v1")
    );

    // Same kind on both, so a kind-keyed lookup could not have told them apart.
    assert_eq!(
        config.get_provider("marvel").unwrap().kind,
        AIProvider::Custom
    );
    assert_eq!(
        config.get_provider("my-vllm").unwrap().kind,
        AIProvider::Custom
    );
}

#[test]
fn test_validate_slug_rejects_anything_that_would_break_a_model_id() {
    let custom = AIProvider::Custom;
    assert!(validate_slug("marvel", custom).is_ok());
    assert!(validate_slug("my-vllm", custom).is_ok());
    assert!(validate_slug("openai", AIProvider::OpenAI).is_ok());
    assert!(validate_slug("7b", custom).is_ok());

    // A `:` in a slug would make `<slug>:<model>` ambiguous.
    assert_eq!(
        validate_slug("mar:vel", custom),
        Err(SlugError::BadChar(':'))
    );
    assert_eq!(
        validate_slug("Marvel", custom),
        Err(SlugError::BadFirstChar)
    );
    assert_eq!(
        validate_slug("-marvel", custom),
        Err(SlugError::BadFirstChar)
    );
    assert_eq!(validate_slug("", custom), Err(SlugError::Empty));
    assert_eq!(
        validate_slug(&"a".repeat(40), custom),
        Err(SlugError::TooLong(40))
    );
    assert!(validate_slug(&"a".repeat(39), custom).is_ok());
}

/// `azure_openai` is the legacy default slug for the Azure kind and does not match the
/// slug pattern — the `_` is not in the character class. It still has to validate: the
/// admin console sends it, without a `slug` field, on every save, for tenants that have
/// never configured Azure. Rejecting it 400s the whole PUT and no AI setting can be
/// saved at all.
#[test]
fn test_a_kind_serde_name_is_always_a_valid_slug_even_when_the_pattern_says_otherwise() {
    for kind in [
        AIProvider::OpenAI,
        AIProvider::Anthropic,
        AIProvider::Google,
        AIProvider::Ollama,
        AIProvider::AzureOpenAI,
        AIProvider::Groq,
        AIProvider::OpenRouter,
        AIProvider::Bedrock,
        AIProvider::Custom,
        AIProvider::Local,
    ] {
        assert!(
            validate_slug(kind.serde_name(), kind).is_ok(),
            "{} is a legacy default slug and must validate",
            kind.serde_name()
        );
    }

    // The exemption is an exact match, not a relaxation of the pattern: a slug that
    // merely looks kind-ish still has to earn its way through.
    assert_eq!(
        validate_slug("azure_openai_eu", AIProvider::AzureOpenAI),
        Err(SlugError::BadChar('_'))
    );
}

/// The exemption belongs to the kind whose name it is, and to no other. `openai`
/// passes the slug pattern on its own, so nothing but this check stops an Anthropic
/// entry being created under it — after which the tenant's stored `openai:gpt-4o`
/// ids resolve, silently and permanently, to Anthropic.
#[test]
fn test_a_slug_naming_a_different_kind_is_refused() {
    assert_eq!(
        validate_slug("openai", AIProvider::Anthropic),
        Err(SlugError::ForeignKindName {
            slug: "openai".to_string(),
            kind: "anthropic",
        })
    );
    // The same slug for the kind it actually names is the legacy default, and fine.
    assert!(validate_slug("openai", AIProvider::OpenAI).is_ok());

    // Including the underscored one, which cannot fall back to the pattern at all.
    assert!(validate_slug("azure_openai", AIProvider::Custom).is_err());
    assert!(validate_slug("azure_openai", AIProvider::AzureOpenAI).is_ok());
}

/// A config written before slugs existed can hold two entries of one kind — a hosted
/// gateway and the tenant's own box, both `custom`, the exact collision slugs were
/// introduced to fix. Both default to slug `custom` on read, and a slug that addresses
/// two entries addresses neither: the merge would update the first while a DELETE
/// removed both. The read has to hand back something coherent.
#[test]
fn test_a_pre_slug_config_with_two_entries_of_one_kind_reads_back_addressable() {
    let legacy = serde_json::json!({
        "tenant_id": "t",
        "providers": [
            {
                "provider": "custom",
                "enabled": true,
                "models": [],
                "api_endpoint": "https://marvel.maravilla.cloud/v1",
                "api_key_encrypted": [1, 2, 3]
            },
            {
                "provider": "custom",
                "enabled": true,
                "models": [],
                "api_endpoint": "http://10.0.0.5:8000/v1",
                "api_key_encrypted": [4, 5, 6]
            }
        ]
    });

    let config: TenantAIConfig = serde_json::from_value(legacy).expect("legacy config must read");

    // Both entries survive — dropping one would lose an endpoint and an encrypted key
    // on a read — and each is reachable under a slug of its own.
    assert_eq!(config.providers.len(), 2);
    let slugs: Vec<&str> = config.providers.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(slugs, vec!["custom", "custom-2"]);

    // The first entry keeps the bare kind name, so every stored `custom:<model>` id
    // still resolves where it always did.
    assert_eq!(
        config
            .get_provider("custom")
            .expect("the first entry keeps the legacy slug")
            .api_endpoint
            .as_deref(),
        Some("https://marvel.maravilla.cloud/v1")
    );
    assert_eq!(
        config
            .get_provider("custom-2")
            .expect("the second entry is addressable too")
            .api_key_encrypted
            .as_deref(),
        Some(&[4u8, 5, 6][..])
    );
}

/// Renaming has to keep clear of slugs further down the list, or the repair recreates
/// the collision it exists to remove.
#[test]
fn test_deduplication_does_not_collide_with_a_slug_defined_later() {
    let stored = serde_json::json!({
        "tenant_id": "t",
        "providers": [
            { "slug": "custom", "provider": "custom", "enabled": true, "models": [] },
            { "slug": "custom", "provider": "custom", "enabled": true, "models": [] },
            { "slug": "custom-2", "provider": "custom", "enabled": true, "models": [] }
        ]
    });

    let config: TenantAIConfig = serde_json::from_value(stored).unwrap();
    let slugs: Vec<&str> = config.providers.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(slugs, vec!["custom", "custom-3", "custom-2"]);
}

#[test]
fn test_a_slug_containing_a_colon_cannot_address_its_own_models() {
    // Belt-and-braces for the rule above: even if such an entry were forced
    // into storage, the model id it implies resolves to nothing.
    let mut config = TenantAIConfig::new("test-tenant".to_string());
    let mut bad = AIProviderConfig::with_slug("mar:vel", AIProvider::Custom);
    bad.models
        .push(AIModelConfig::new("smart".to_string(), "Smart".to_string()));
    config.providers.push(bad);

    assert!(validate_slug("mar:vel", AIProvider::Custom).is_err());
    assert!(config.get_model("mar:vel:smart").is_none());
}
