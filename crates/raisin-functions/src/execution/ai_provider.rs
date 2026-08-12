// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Shared AI provider creation logic.
//!
//! This module provides the single source of truth for creating AI provider instances.
//! Both HTTP and trigger execution paths must use these functions to ensure consistent
//! behavior across all AI operations.

use raisin_ai::{
    config::{AIProvider, AIProviderConfig, TenantAIConfig},
    crypto::ApiKeyEncryptor,
    provider::AIProviderTrait,
    providers::{
        AnthropicProvider, BedrockProvider, GroqProvider, OllamaProvider, OpenAIProvider,
        OpenRouterProvider,
    },
    TenantAIConfigStore,
};

/// Creates an AI provider instance from tenant configuration.
///
/// This is the single source of truth for provider creation.
/// Both HTTP and trigger paths must use this function.
///
/// # Arguments
///
/// * `config_store` - The tenant AI configuration store
/// * `tenant_id` - The tenant identifier
/// * `model_id` - The model identifier to look up
///
/// # Returns
///
/// A boxed AI provider trait object ready to make completion calls.
///
/// # Errors
///
/// Returns an error if:
/// - Failed to load tenant AI config
/// - Model not found in tenant configuration
/// - Provider is disabled
/// - API key decryption fails
/// - Provider type not supported
pub async fn create_provider_for_model(
    config_store: &dyn TenantAIConfigStore,
    tenant_id: &str,
    model_id: &str,
) -> Result<Box<dyn AIProviderTrait>, raisin_error::Error> {
    // Special case: local models work without tenant configuration
    // They use on-device Candle inference, no API keys needed
    if model_id.starts_with("local:") {
        // Check if local provider is explicitly disabled in tenant config
        if let Ok(config) = config_store.get_config(tenant_id).await {
            // Matched on KIND, not slug, on purpose: the `local:` prefix here is a
            // built-in route to in-process Candle inference, not a reference to a
            // tenant entry, so "has this tenant switched local inference off"
            // is a question about the kind however the entry is slugged.
            if let Some(local_provider) = config
                .providers
                .iter()
                .find(|p| p.kind == AIProvider::Local)
            {
                if !local_provider.enabled {
                    return Err(raisin_error::Error::Backend(
                        "Local AI models are disabled for this tenant".to_string(),
                    ));
                }
            }
        }

        let models_dir =
            std::env::var("RAISIN_MODELS_DIR").unwrap_or_else(|_| "./models".to_string());
        tracing::debug!(
            model_id = %model_id,
            models_dir = %models_dir,
            "Creating local Candle provider (no tenant config required)"
        );
        return Ok(Box::new(raisin_ai::providers::LocalCandleProvider::new(
            models_dir,
        )));
    }

    // 1. Load tenant config
    let config = config_store
        .get_config(tenant_id)
        .await
        .map_err(|e| raisin_error::Error::Backend(format!("Failed to get AI config: {}", e)))?;

    // 2. Dynamic model support for providers that accept arbitrary models.
    // These providers don't require explicit model registration - just a configured provider.
    // If model_id is "ollama:mistral", we find the entry slugged `ollama` and use it directly.
    if let Some(provider_config) = dynamic_provider_for_model(&config, model_id) {
        if !provider_config.enabled {
            return Err(raisin_error::Error::Backend(format!(
                "AI provider '{}' is disabled",
                provider_config.slug
            )));
        }

        tracing::debug!(
            model_id = %model_id,
            provider = %provider_config.slug,
            kind = ?provider_config.kind,
            "Using dynamic model (no explicit registration required)"
        );

        // Provider found and enabled - create it directly
        let api_key = decrypt_api_key_if_needed(provider_config)?;
        return create_provider_instance(provider_config, api_key.as_deref());
    }

    // 3. Fallback: Strict model lookup for other cases (backward compatibility).
    //
    // An unresolved prefix lands here rather than erroring above, because a prefix
    // this tenant has not slugged is indistinguishable from a model name that just
    // happens to contain a colon (`qwen2.5-coder:latest`). The configured slugs go
    // into the message: "provider not configured" used to be a distinct error, and
    // the operator still needs to be told which names would have worked.
    let (provider_config, _model_config) = config.get_model(model_id).ok_or_else(|| {
        let slugs = config
            .providers
            .iter()
            .map(|p| p.slug.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        raisin_error::Error::NotFound(format!(
            "Model '{}' not found in tenant configuration. Configured providers: [{}]. \
             Add one in Admin Console > AI Settings.",
            model_id, slugs
        ))
    })?;

    // 4. Check if enabled
    if !provider_config.enabled {
        return Err(raisin_error::Error::Backend(format!(
            "Provider '{}' is disabled",
            provider_config.slug
        )));
    }

    // 5. Decrypt API key if needed (returns Option<String>)
    let api_key = decrypt_api_key_if_needed(provider_config)?;

    // 6. Create provider - use provider_config.api_endpoint directly!
    //    Let each provider's new() handle its own defaults.
    create_provider_instance(provider_config, api_key.as_deref())
}

/// Decrypts the API key from provider config.
///
/// Returns Ok(Some(key)) if API key is present and decrypted successfully.
/// Returns Ok(None) if no API key is configured and provider doesn't require one.
/// Returns Err if provider requires API key but none is configured, or decryption fails.
fn decrypt_api_key_if_needed(
    provider_config: &AIProviderConfig,
) -> Result<Option<String>, raisin_error::Error> {
    match &provider_config.api_key_encrypted {
        Some(encrypted) => {
            // Decrypt the key if present
            let master_key = get_master_key()?;
            let encryptor = ApiKeyEncryptor::new(&master_key);

            let key = encryptor.decrypt(encrypted).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to decrypt API key: {}", e))
            })?;

            Ok(Some(key))
        }
        None => {
            // No encrypted key configured
            if provider_config.kind.requires_api_key() {
                Err(raisin_error::Error::Backend(format!(
                    "API key not configured for provider '{}' ({:?})",
                    provider_config.slug, provider_config.kind
                )))
            } else {
                Ok(None)
            }
        }
    }
}

/// Whether a provider KIND accepts any model id without that model being registered in
/// the tenant's AI config — i.e. `<slug>:anything` resolves as long as the entry behind
/// `<slug>` is configured and enabled.
///
/// Takes the kind, not the model-id prefix: the prefix is a per-tenant slug, and a
/// tenant that slugs its gateway `my-vllm` would otherwise never match a name on this
/// list and would fall through to the strict model lookup that a gateway deliberately
/// leaves empty. Resolve slug -> entry -> kind first; [`dynamic_provider_for_model`]
/// does exactly that.
///
/// `Custom` is on this list so an agent node can name a gateway model with nothing but
/// two strings — `provider: marvel`, `model: maravilla/smart` — and no prior
/// registration. A gateway publishes its own catalogue and adds to it over time;
/// requiring each alias to be registered in tenant config first would mean
/// re-provisioning every tenant before anyone could use a newly published model.
///
/// The cost of being on this list is that per-model tenant config (`default_temperature`,
/// `default_max_tokens`) is bypassed, because there is no registered model to read it
/// from. Callers that care supply their own — an agent node carries both.
fn supports_dynamic_models(kind: AIProvider) -> bool {
    matches!(
        kind,
        AIProvider::Ollama
            | AIProvider::OpenAI
            | AIProvider::Anthropic
            | AIProvider::Groq
            | AIProvider::OpenRouter
            | AIProvider::Custom
    )
}

/// Resolves `<slug>:<model>` to the configured entry that can serve it without the
/// model being registered, or `None` if there is no such entry.
///
/// `parse_model_id` is what decides whether the part before the colon is a prefix at
/// all: it only says yes for a slug this tenant actually has, so `qwen2.5-coder:latest`
/// stays a bare model name.
fn dynamic_provider_for_model<'a>(
    config: &'a TenantAIConfig,
    model_id: &str,
) -> Option<&'a AIProviderConfig> {
    let (slug, _model_name) = config.parse_model_id(model_id);
    let entry = config.get_provider(slug?)?;
    supports_dynamic_models(entry.kind).then_some(entry)
}

/// Creates a provider instance based on the provider type and configuration.
///
/// GOLDEN STANDARD: Uses provider_config.api_endpoint directly.
/// Let each provider's new() handle its correct default endpoint.
fn create_provider_instance(
    provider_config: &AIProviderConfig,
    api_key: Option<&str>,
) -> Result<Box<dyn AIProviderTrait>, raisin_error::Error> {
    match provider_config.kind {
        AIProvider::OpenAI => {
            let key = api_key.ok_or_else(|| {
                raisin_error::Error::Backend("OpenAI requires API key".to_string())
            })?;
            if let Some(endpoint) = &provider_config.api_endpoint {
                Ok(Box::new(OpenAIProvider::with_base_url(key, endpoint)))
            } else {
                Ok(Box::new(OpenAIProvider::new(key)))
            }
        }
        AIProvider::Anthropic => {
            let key = api_key.ok_or_else(|| {
                raisin_error::Error::Backend("Anthropic requires API key".to_string())
            })?;
            if let Some(endpoint) = &provider_config.api_endpoint {
                Ok(Box::new(AnthropicProvider::with_base_url(key, endpoint)))
            } else {
                Ok(Box::new(AnthropicProvider::new(key)))
            }
        }
        AIProvider::Ollama => {
            // Ollama: endpoint and API key are both optional
            let mut provider = if let Some(endpoint) = &provider_config.api_endpoint {
                OllamaProvider::with_base_url(endpoint)
            } else {
                // Uses correct default with /api suffix
                OllamaProvider::new()
            };

            // Add API key if configured (for authenticated Ollama endpoints)
            if let Some(key) = api_key {
                provider = provider.with_api_key(key);
            }

            Ok(Box::new(provider))
        }
        AIProvider::AzureOpenAI => {
            let key = api_key.ok_or_else(|| {
                raisin_error::Error::Backend("Azure OpenAI requires API key".to_string())
            })?;
            let endpoint = provider_config.api_endpoint.as_ref().ok_or_else(|| {
                raisin_error::Error::Backend("Azure OpenAI requires custom endpoint".to_string())
            })?;
            Ok(Box::new(OpenAIProvider::with_base_url(key, endpoint)))
        }
        AIProvider::Groq => {
            let key = api_key
                .ok_or_else(|| raisin_error::Error::Backend("Groq requires API key".to_string()))?;
            if let Some(endpoint) = &provider_config.api_endpoint {
                Ok(Box::new(GroqProvider::with_base_url(key, endpoint)))
            } else {
                Ok(Box::new(GroqProvider::new(key)))
            }
        }
        AIProvider::OpenRouter => {
            let key = api_key.ok_or_else(|| {
                raisin_error::Error::Backend("OpenRouter requires API key".to_string())
            })?;
            if let Some(endpoint) = &provider_config.api_endpoint {
                Ok(Box::new(OpenRouterProvider::with_base_url(key, endpoint)))
            } else {
                Ok(Box::new(OpenRouterProvider::new(key)))
            }
        }
        AIProvider::Bedrock => {
            // Bedrock uses api_endpoint as region (e.g., "us-east-1")
            // and api_key as "access_key_id:secret_access_key"
            let key = api_key.ok_or_else(|| {
                raisin_error::Error::Backend("AWS Bedrock requires API key".to_string())
            })?;
            let region = provider_config.api_endpoint.as_ref().ok_or_else(|| {
                raisin_error::Error::Backend(
                    "AWS Bedrock requires region in api_endpoint (e.g., 'us-east-1')".to_string(),
                )
            })?;

            // Parse api_key as "access_key_id:secret_access_key"
            let parts: Vec<&str> = key.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(raisin_error::Error::Backend(
                    "AWS Bedrock api_key must be in format 'access_key_id:secret_access_key'"
                        .to_string(),
                ));
            }
            let access_key_id = parts[0];
            let secret_access_key = parts[1];

            Ok(Box::new(BedrockProvider::new(
                region,
                access_key_id,
                secret_access_key,
            )))
        }
        AIProvider::Local => {
            // Local Candle models - get models directory from environment or use default
            let models_dir =
                std::env::var("RAISIN_MODELS_DIR").unwrap_or_else(|_| "./models".to_string());
            Ok(Box::new(raisin_ai::providers::LocalCandleProvider::new(
                models_dir,
            )))
        }
        AIProvider::Custom => {
            // Any OpenAI-compatible `/chat/completions` endpoint: a self-hosted vLLM or
            // llama.cpp server, or a gateway sitting in front of one.
            //
            // `GroqProvider` *is* that client — the only Groq-specific things about it
            // are its default base URL and a model-name heuristic for tool support, and
            // a supplied endpoint replaces the first. Reusing it avoids maintaining a
            // second copy of the same protocol.
            //
            // The endpoint is required rather than defaulted: there is no sensible
            // default host for "custom", and guessing one would send a tenant's traffic
            // somewhere they never asked for.
            //
            // The key is *not* required, matching `AIProvider::requires_api_key()` — a
            // self-hosted endpoint on a private network legitimately has none. An
            // endpoint that does want one answers a keyless request with its own 401,
            // which says more than a generic config error could.
            let key = api_key.unwrap_or_default();
            let endpoint = provider_config.api_endpoint.as_ref().ok_or_else(|| {
                raisin_error::Error::Backend(
                    "Custom provider requires api_endpoint (the OpenAI-compatible base \
                     URL, e.g. https://host/v1)"
                        .to_string(),
                )
            })?;
            Ok(Box::new(GroqProvider::with_base_url(key, endpoint)))
        }
        AIProvider::Google => Err(raisin_error::Error::Backend(format!(
            "Provider {:?} not yet supported",
            provider_config.kind
        ))),
    }
}

/// Gets the master encryption key from environment variable.
///
/// Delegates to the shared loader, which reads `RAISIN_MASTER_KEY` only and
/// hard-errors if it is missing/invalid (no fallback).
fn get_master_key() -> Result<[u8; 32], raisin_error::Error> {
    raisin_crypto::master_key()
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_ai::config::AIProviderConfig;

    fn config(kind: AIProvider, endpoint: Option<&str>) -> AIProviderConfig {
        entry(kind.serde_name(), kind, endpoint)
    }

    fn entry(slug: &str, kind: AIProvider, endpoint: Option<&str>) -> AIProviderConfig {
        AIProviderConfig {
            api_endpoint: endpoint.map(str::to_string),
            ..AIProviderConfig::with_slug(slug, kind)
        }
    }

    fn tenant(entries: Vec<AIProviderConfig>) -> TenantAIConfig {
        TenantAIConfig {
            providers: entries,
            ..TenantAIConfig::new("t".to_string())
        }
    }

    /// The whole point of `Custom`: reach an OpenAI-compatible endpoint that this build
    /// has never heard of. Before this it returned "not yet supported", which forced
    /// tenants to masquerade as `groq` while pointing somewhere else entirely.
    #[test]
    fn custom_builds_against_a_supplied_endpoint() {
        let cfg = config(
            AIProvider::Custom,
            Some("https://marvel.maravilla.cloud/v1"),
        );
        assert!(create_provider_instance(&cfg, Some("mrv_key")).is_ok());
    }

    /// There is no sensible default host for "custom"; guessing one would send a
    /// tenant's traffic somewhere they never asked for.
    #[test]
    fn custom_without_an_endpoint_is_refused_with_a_usable_message() {
        let cfg = config(AIProvider::Custom, None);
        let err = create_provider_instance(&cfg, Some("k"))
            .err()
            .expect("an endpoint is required")
            .to_string();
        assert!(err.contains("api_endpoint"), "got: {err}");
    }

    /// Matches `AIProvider::requires_api_key()`, which reports false for Custom — a
    /// self-hosted endpoint on a private network legitimately has no key.
    #[test]
    fn custom_does_not_require_an_api_key() {
        let cfg = config(AIProvider::Custom, Some("http://10.0.0.5:8000/v1"));
        assert!(create_provider_instance(&cfg, None).is_ok());
    }

    #[test]
    fn google_is_still_unsupported() {
        let cfg = config(AIProvider::Google, Some("https://example.test"));
        assert!(create_provider_instance(&cfg, Some("k")).is_err());
    }

    /// An agent node written as `provider: marvel` / `model: maravilla/smart` is joined
    /// into `marvel:maravilla/smart` before resolution. Without `Custom` on the dynamic
    /// list that falls through to a strict lookup against the tenant's registered models,
    /// which a gateway deliberately leaves empty — so every such agent fails with
    /// NotFound however correct its YAML is.
    #[test]
    fn custom_resolves_models_that_were_never_registered() {
        let cfg = tenant(vec![entry(
            "marvel",
            AIProvider::Custom,
            Some("https://marvel.maravilla.cloud/v1"),
        )]);
        let resolved = dynamic_provider_for_model(&cfg, "marvel:maravilla/smart")
            .expect("a Custom entry serves unregistered models");
        assert_eq!(resolved.slug, "marvel");
        assert!(resolved.models.is_empty());
    }

    /// The dynamic decision is made on the entry's KIND, so it survives any slug the
    /// tenant picks. Matching the slug against the kind names instead would send
    /// `my-vllm:foo` down the strict-lookup path and fail it.
    #[test]
    fn a_slug_that_is_not_a_kind_name_still_takes_the_dynamic_path() {
        let cfg = tenant(vec![entry(
            "my-vllm",
            AIProvider::Custom,
            Some("http://10.0.0.5:8000/v1"),
        )]);
        assert!(dynamic_provider_for_model(&cfg, "my-vllm:llama-3.1-70b").is_some());
    }

    /// A gateway alias contains a `/`, not a `:`, so the split that qualifies the id
    /// takes the provider prefix and leaves the alias whole.
    #[test]
    fn a_gateway_alias_survives_prefix_qualification() {
        let cfg = tenant(vec![entry(
            "marvel",
            AIProvider::Custom,
            Some("https://marvel.maravilla.cloud/v1"),
        )]);
        let (slug, model) = cfg.parse_model_id("marvel:maravilla/smart");
        assert_eq!(slug, Some("marvel"));
        assert_eq!(model, "maravilla/smart");
    }

    /// The prefix is a slug, not a kind name: a kind this tenant has not configured is
    /// not a prefix at all. `anthropic:claude` with no `anthropic` entry must NOT
    /// resolve to some other Anthropic-kind entry the tenant slugged differently.
    #[test]
    fn an_unconfigured_kind_name_is_not_a_prefix() {
        let cfg = tenant(vec![entry("claude-eu", AIProvider::Anthropic, None)]);
        assert!(dynamic_provider_for_model(&cfg, "anthropic:claude-sonnet-4").is_none());
        assert!(dynamic_provider_for_model(&cfg, "claude-eu:claude-sonnet-4").is_some());
    }

    /// A model name that merely contains a colon (`qwen2.5-coder:latest`) is not a
    /// prefixed id, and must not be mistaken for one — otherwise Ollama's own naming
    /// convention becomes unroutable.
    #[test]
    fn a_colon_inside_a_model_name_is_not_a_prefix() {
        let cfg = tenant(vec![entry("ollama", AIProvider::Ollama, None)]);
        assert!(dynamic_provider_for_model(&cfg, "qwen2.5-coder:latest").is_none());
        assert!(dynamic_provider_for_model(&cfg, "ollama:qwen2.5-coder:latest").is_some());
    }

    /// Guard against a kind quietly joining the list: everything here bypasses
    /// per-model tenant config, which is a deliberate trade, not a default.
    #[test]
    fn the_dynamic_list_is_exactly_what_we_intend() {
        for k in [
            AIProvider::Ollama,
            AIProvider::OpenAI,
            AIProvider::Anthropic,
            AIProvider::Groq,
            AIProvider::OpenRouter,
            AIProvider::Custom,
        ] {
            assert!(supports_dynamic_models(k), "{k:?} should be dynamic");
        }
        for k in [
            AIProvider::AzureOpenAI,
            AIProvider::Bedrock,
            AIProvider::Google,
            AIProvider::Local,
        ] {
            assert!(!supports_dynamic_models(k), "{k:?} should not be dynamic");
        }
    }
}
