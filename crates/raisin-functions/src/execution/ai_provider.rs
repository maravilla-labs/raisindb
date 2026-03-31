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
    config::{AIProvider, AIProviderConfig},
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
            if let Some(local_provider) = config
                .providers
                .iter()
                .find(|p| p.provider == AIProvider::Local)
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

    // 2. Dynamic model support for providers that accept arbitrary models
    // These providers don't require explicit model registration - just a configured provider.
    // If model_id is "ollama:mistral", we find the ollama provider and use it directly.
    if let Some((prefix, _model_name)) = model_id.split_once(':') {
        // Providers that support any model without explicit registration
        let supports_dynamic_models = matches!(
            prefix,
            "ollama" | "openai" | "anthropic" | "groq" | "openrouter"
        );

        if supports_dynamic_models {
            if let Some(target_provider) = AIProvider::from_serde_name(prefix) {
                if let Some(provider_config) = config
                    .providers
                    .iter()
                    .find(|p| p.provider == target_provider)
                {
                    if !provider_config.enabled {
                        return Err(raisin_error::Error::Backend(format!(
                            "AI provider '{}' is disabled",
                            prefix
                        )));
                    }

                    tracing::debug!(
                        model_id = %model_id,
                        provider = %prefix,
                        "Using dynamic model (no explicit registration required)"
                    );

                    // Provider found and enabled - create it directly
                    let api_key = decrypt_api_key_if_needed(provider_config)?;
                    return create_provider_instance(provider_config, api_key.as_deref());
                }
            }

            // Provider prefix recognized but not configured
            return Err(raisin_error::Error::NotFound(format!(
                "AI provider '{}' not configured for tenant. Add it in Admin Console > AI Settings.",
                prefix
            )));
        }
    }

    // 3. Fallback: Strict model lookup for other cases (backward compatibility)
    let (provider_config, _model_config) = config.get_model(model_id).ok_or_else(|| {
        raisin_error::Error::NotFound(format!(
            "Model '{}' not found in tenant configuration",
            model_id
        ))
    })?;

    // 4. Check if enabled
    if !provider_config.enabled {
        return Err(raisin_error::Error::Backend(format!(
            "Provider {:?} is disabled",
            provider_config.provider
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
            if provider_config.provider.requires_api_key() {
                Err(raisin_error::Error::Backend(format!(
                    "API key not configured for provider {:?}",
                    provider_config.provider
                )))
            } else {
                Ok(None)
            }
        }
    }
}

/// Creates a provider instance based on the provider type and configuration.
///
/// GOLDEN STANDARD: Uses provider_config.api_endpoint directly.
/// Let each provider's new() handle its correct default endpoint.
fn create_provider_instance(
    provider_config: &AIProviderConfig,
    api_key: Option<&str>,
) -> Result<Box<dyn AIProviderTrait>, raisin_error::Error> {
    match provider_config.provider {
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
        AIProvider::Google | AIProvider::Custom => Err(raisin_error::Error::Backend(format!(
            "Provider {:?} not yet supported",
            provider_config.provider
        ))),
    }
}

/// Gets the master encryption key from environment variable.
fn get_master_key() -> Result<[u8; 32], raisin_error::Error> {
    let hex = std::env::var("RAISIN_MASTER_KEY").map_err(|_| {
        raisin_error::Error::Backend("RAISIN_MASTER_KEY environment variable not set".to_string())
    })?;

    let bytes = hex::decode(&hex)
        .map_err(|e| raisin_error::Error::Backend(format!("Invalid RAISIN_MASTER_KEY: {}", e)))?;

    bytes
        .try_into()
        .map_err(|_| raisin_error::Error::Backend("RAISIN_MASTER_KEY must be 32 bytes".to_string()))
}
