// SPDX-License-Identifier: BSL-1.1

//! AI callback builders for the function API.
//!
//! Constructs the AI-related callbacks (completion, list-models,
//! get-default-model) used by `RaisinFunctionApi` for JavaScript/Starlark
//! functions that call AI services.

use std::sync::Arc;

use super::file_helpers::get_master_encryption_key;

#[cfg(feature = "storage-rocksdb")]
use raisin_ai::TenantAIConfigStore;
#[cfg(feature = "storage-rocksdb")]
use raisin_functions::execution::ai_provider::create_provider_for_model;

/// Async callback that takes a JSON request and returns a JSON result.
#[cfg(feature = "storage-rocksdb")]
type AiCompletionCallback = Arc<
    dyn Fn(
            serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, raisin_error::Error>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

/// Async callback that returns a list of available AI models.
#[cfg(feature = "storage-rocksdb")]
type AiListModelsCallback = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Vec<serde_json::Value>, raisin_error::Error>,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

/// Async callback that takes a use-case string and returns the default model ID.
#[cfg(feature = "storage-rocksdb")]
type AiGetDefaultModelCallback = Arc<
    dyn Fn(
            String,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Option<String>, raisin_error::Error>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

/// Build the AI completion callback.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn build_ai_completion(
    storage: Arc<crate::state::Store>,
    tenant: String,
) -> AiCompletionCallback {
    Arc::new(move |request: serde_json::Value| {
        let storage = storage.clone();
        let tenant = tenant.clone();
        Box::pin(async move {
            // Parse the completion request
            let req: raisin_ai::types::CompletionRequest = serde_json::from_value(request)
                .map_err(|e| {
                    raisin_error::Error::Validation(format!("Invalid completion request: {}", e))
                })?;

            // Get AI config repository and use shared factory
            let ai_config_repo = storage.tenant_ai_config_repository();
            let provider = create_provider_for_model(&ai_config_repo, &tenant, &req.model).await?;

            // Call the provider's completion API
            let response = provider.complete(req).await.map_err(|e| {
                raisin_error::Error::Backend(format!("AI completion failed: {}", e))
            })?;

            // Convert response to JSON
            serde_json::to_value(response).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to serialize response: {}", e))
            })
        })
    })
}

/// Build the AI list-models callback.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn build_ai_list_models(
    storage: Arc<crate::state::Store>,
    tenant: String,
) -> AiListModelsCallback {
    Arc::new(move || {
        let storage = storage.clone();
        let tenant = tenant.clone();
        Box::pin(async move {
            // Get AI config repository
            let ai_config_repo = storage.tenant_ai_config_repository();

            // Load tenant AI configuration
            let config = ai_config_repo.get_config(&tenant).await.map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to load AI config: {}", e))
            })?;

            // Get master key for decryption
            let master_key = get_master_encryption_key()?;
            let encryptor = raisin_ai::crypto::ApiKeyEncryptor::new(&master_key);

            let mut all_models = Vec::new();

            // Iterate through enabled providers and collect their models
            for provider_config in config.providers.iter().filter(|p| p.enabled) {
                // Decrypt API key if present
                let api_key = if let Some(encrypted) = &provider_config.api_key_encrypted {
                    encryptor.decrypt(encrypted).map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to decrypt API key: {}", e))
                    })?
                } else if !provider_config.provider.requires_api_key() {
                    String::new()
                } else {
                    // Skip this provider if no API key is configured
                    continue;
                };

                // Create the appropriate provider instance
                let provider: Box<dyn raisin_ai::provider::AIProviderTrait> =
                    match provider_config.provider {
                        raisin_ai::config::AIProvider::OpenAI => {
                            if let Some(endpoint) = &provider_config.api_endpoint {
                                Box::new(raisin_ai::providers::OpenAIProvider::with_base_url(
                                    &api_key, endpoint,
                                ))
                            } else {
                                Box::new(raisin_ai::providers::OpenAIProvider::new(&api_key))
                            }
                        }
                        raisin_ai::config::AIProvider::Anthropic => {
                            if let Some(endpoint) = &provider_config.api_endpoint {
                                Box::new(raisin_ai::providers::AnthropicProvider::with_base_url(
                                    &api_key, endpoint,
                                ))
                            } else {
                                Box::new(raisin_ai::providers::AnthropicProvider::new(&api_key))
                            }
                        }
                        raisin_ai::config::AIProvider::Ollama => {
                            if let Some(endpoint) = &provider_config.api_endpoint {
                                Box::new(raisin_ai::providers::OllamaProvider::with_base_url(
                                    endpoint,
                                ))
                            } else {
                                Box::new(raisin_ai::providers::OllamaProvider::new())
                            }
                        }
                        _ => {
                            // Skip unsupported providers
                            continue;
                        }
                    };

                // Get models from this provider
                match provider.list_available_models().await {
                    Ok(models) => {
                        for model in models {
                            // Convert ModelInfo to JSON
                            all_models.push(serde_json::json!({
                                "id": model.id,
                                "name": model.name,
                                "provider": provider.provider_name(),
                                "capabilities": {
                                    "chat": model.capabilities.chat,
                                    "streaming": model.capabilities.streaming,
                                    "tools": model.capabilities.tools,
                                    "embeddings": model.capabilities.embeddings,
                                    "vision": model.capabilities.vision,
                                }
                            }));
                        }
                    }
                    Err(e) => {
                        // Log error but continue with other providers
                        tracing::warn!(
                            "Failed to list models from provider {}: {}",
                            provider.provider_name(),
                            e
                        );
                    }
                }
            }

            Ok(all_models)
        })
    })
}

/// Build the AI get-default-model callback.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn build_ai_get_default_model(
    storage: Arc<crate::state::Store>,
    tenant: String,
) -> AiGetDefaultModelCallback {
    Arc::new(move |use_case: String| {
        let storage = storage.clone();
        let tenant = tenant.clone();
        Box::pin(async move {
            // Get AI config repository
            let ai_config_repo = storage.tenant_ai_config_repository();

            // Load tenant AI configuration
            let config = ai_config_repo.get_config(&tenant).await.map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to load AI config: {}", e))
            })?;

            // Parse the use case
            let ai_use_case = match use_case.to_lowercase().as_str() {
                "chat" => raisin_ai::config::AIUseCase::Chat,
                "completion" => raisin_ai::config::AIUseCase::Completion,
                "agent" => raisin_ai::config::AIUseCase::Agent,
                "embedding" => raisin_ai::config::AIUseCase::Embedding,
                "classification" => raisin_ai::config::AIUseCase::Classification,
                _ => {
                    return Err(raisin_error::Error::Validation(format!(
                        "Unknown use case: {}",
                        use_case
                    )))
                }
            };

            // Find the default model for this use case
            for provider_config in config.providers.iter().filter(|p| p.enabled) {
                if let Some(model_config) = provider_config.get_default_model(ai_use_case) {
                    return Ok(Some(model_config.model_id.clone()));
                }
            }

            // No default model found
            Ok(None)
        })
    })
}
