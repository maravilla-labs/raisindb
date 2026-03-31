// SPDX-License-Identifier: BSL-1.1

//! Model listing and discovery handlers.
//!
//! Endpoints for listing all available models (with optional refresh from
//! provider APIs), filtering models by use case, and fetching models from
//! individual providers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};

use raisin_ai::{
    config::{AIModelConfig, AIProvider, AIUseCase},
    crypto::ApiKeyEncryptor,
    provider::AIProviderTrait,
    providers::{
        AnthropicProvider, AzureOpenAIProvider, BedrockProvider, GeminiProvider, GroqProvider,
        OllamaProvider, OpenAIProvider, OpenRouterProvider,
    },
    storage::TenantAIConfigStore,
};

use crate::state::AppState;

use super::types::{ErrorResponse, ListModelsQuery, ModelInfo, ModelsResponse};

/// Get all available models from configured providers.
///
/// GET /api/tenants/{tenant_id}/ai/models?provider=ollama&refresh=true
///
/// Query parameters:
/// - provider: Optional filter by provider (e.g., "ollama", "openai")
/// - refresh: If true, fetch models from provider APIs and update config
///
/// Returns cached models by default. Use refresh=true to discover new models.
#[axum::debug_handler]
pub async fn list_all_models(
    Path(tenant_id): Path<String>,
    Query(query): Query<ListModelsQuery>,
    State(state): State<AppState>,
) -> Result<Json<ModelsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo_impl = state.storage().tenant_ai_config_repository();

    // Get current config
    let mut config = match repo_impl.get_config(&tenant_id).await {
        Ok(config) => config,
        Err(raisin_ai::storage::StorageError::NotFound(_)) => {
            return Ok(Json(ModelsResponse { models: Vec::new() }));
        }
        Err(e) => {
            tracing::error!("Failed to get AI config for {}: {}", tenant_id, e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ));
        }
    };

    // If refresh requested, fetch models from provider APIs
    if query.refresh {
        let master_key = state.get_master_key().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Master key not configured: {}", e),
                }),
            )
        })?;
        let encryptor = ApiKeyEncryptor::new(&master_key);

        for provider_config in &mut config.providers {
            // Skip if provider filter is set and doesn't match
            if let Some(ref filter_provider) = query.provider {
                if &provider_config.provider != filter_provider {
                    continue;
                }
            }

            // Skip disabled providers
            if !provider_config.enabled {
                continue;
            }

            // Decrypt API key if present
            let api_key = provider_config
                .api_key_encrypted
                .as_ref()
                .and_then(|encrypted| encryptor.decrypt(encrypted).ok());

            let endpoint = provider_config.api_endpoint.as_deref();

            // Fetch models from provider
            let fetched_models = match fetch_models_from_provider(
                provider_config.provider,
                api_key.as_deref(),
                endpoint,
            )
            .await
            {
                Ok(models) => models,
                Err(e) => {
                    tracing::warn!(
                        "Failed to fetch models from {:?} for tenant {}: {}",
                        provider_config.provider,
                        tenant_id,
                        e
                    );
                    continue; // Skip this provider but continue with others
                }
            };

            // Update provider config with fetched models
            provider_config.models = fetched_models
                .into_iter()
                .map(|m| {
                    // Infer use cases from capabilities
                    let mut use_cases = Vec::new();
                    if m.capabilities.chat {
                        use_cases.push(AIUseCase::Chat);
                        use_cases.push(AIUseCase::Completion);
                    }
                    if m.capabilities.embeddings {
                        use_cases.push(AIUseCase::Embedding);
                    }
                    if m.capabilities.tools {
                        use_cases.push(AIUseCase::Agent);
                    }
                    // Default to chat if nothing detected
                    if use_cases.is_empty() {
                        use_cases.push(AIUseCase::Chat);
                    }

                    AIModelConfig {
                        model_id: m.id.clone(),
                        display_name: m.name.clone(),
                        use_cases,
                        default_temperature: 0.7,
                        default_max_tokens: m.max_output_tokens.unwrap_or(4096),
                        is_default: false,
                        metadata: m.metadata,
                    }
                })
                .collect();
        }

        // Save updated config
        if let Err(e) = repo_impl.set_config(&config).await {
            tracing::error!(
                "Failed to save refreshed AI config for {}: {}",
                tenant_id,
                e
            );
            // Don't fail the request, just log the error
        }
    }

    // Build response from config
    let mut models = Vec::new();
    for provider_config in config.providers {
        // Skip if provider filter is set and doesn't match
        if let Some(ref filter_provider) = query.provider {
            if &provider_config.provider != filter_provider {
                continue;
            }
        }

        for model in provider_config.models {
            models.push(ModelInfo {
                model_id: model.model_id,
                display_name: model.display_name,
                provider: provider_config.provider,
                use_cases: model.use_cases,
                default_temperature: model.default_temperature,
                default_max_tokens: model.default_max_tokens,
            });
        }
    }

    Ok(Json(ModelsResponse { models }))
}

/// Fetch models from a provider's API.
pub(super) async fn fetch_models_from_provider(
    provider: AIProvider,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Vec<raisin_ai::model_cache::ModelInfo>, String> {
    match provider {
        AIProvider::OpenAI => {
            let p = match endpoint {
                Some(url) => OpenAIProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => OpenAIProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models()
                .await
                .map_err(|e| format!("OpenAI error: {}", e))
        }
        AIProvider::Anthropic => {
            let p = AnthropicProvider::new(api_key.unwrap_or_default());
            p.list_available_models()
                .await
                .map_err(|e| format!("Anthropic error: {}", e))
        }
        AIProvider::Google => {
            let p = match endpoint {
                Some(url) => GeminiProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => GeminiProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models()
                .await
                .map_err(|e| format!("Google error: {}", e))
        }
        AIProvider::AzureOpenAI => {
            if let Some(azure_endpoint) = endpoint {
                let p = AzureOpenAIProvider::new(api_key.unwrap_or_default(), azure_endpoint);
                p.list_available_models()
                    .await
                    .map_err(|e| format!("Azure OpenAI error: {}", e))
            } else {
                Err("Azure OpenAI requires a custom endpoint".to_string())
            }
        }
        AIProvider::Ollama => {
            let mut p = match endpoint {
                Some(url) => OllamaProvider::with_base_url(url),
                None => OllamaProvider::new(),
            };
            if let Some(key) = api_key {
                p = p.with_api_key(key);
            }
            p.list_available_models()
                .await
                .map_err(|e| format!("Ollama error: {}", e))
        }
        AIProvider::Groq => {
            let p = match endpoint {
                Some(url) => GroqProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => GroqProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models()
                .await
                .map_err(|e| format!("Groq error: {}", e))
        }
        AIProvider::OpenRouter => {
            let p = match endpoint {
                Some(url) => OpenRouterProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => OpenRouterProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models()
                .await
                .map_err(|e| format!("OpenRouter error: {}", e))
        }
        AIProvider::Bedrock => {
            if let Some(region) = endpoint {
                let key = api_key.unwrap_or_default();
                let parts: Vec<&str> = key.splitn(2, ':').collect();
                if parts.len() == 2 {
                    let p = BedrockProvider::new(region, parts[0], parts[1]);
                    p.list_available_models()
                        .await
                        .map_err(|e| format!("Bedrock error: {}", e))
                } else {
                    Err(
                        "AWS Bedrock api_key must be in format 'access_key_id:secret_access_key'"
                            .to_string(),
                    )
                }
            } else {
                Err("AWS Bedrock requires region in endpoint".to_string())
            }
        }
        AIProvider::Custom => Err("Custom provider not supported for model discovery".to_string()),
        AIProvider::Local => {
            // Local Candle models are defined statically
            Ok(vec![
                raisin_ai::model_cache::ModelInfo {
                    id: "moondream".to_string(),
                    name: "Moondream (Vision-Language)".to_string(),
                    description: Some(
                        "Local vision-language model for image captioning and VQA".to_string(),
                    ),
                    capabilities: raisin_ai::model_cache::ModelCapabilities {
                        chat: true,
                        streaming: false,
                        tools: false,
                        embeddings: false,
                        vision: true,
                    },
                    context_window: Some(2048),
                    max_output_tokens: Some(512),
                    available: true,
                    metadata: None,
                },
                raisin_ai::model_cache::ModelInfo {
                    id: "blip".to_string(),
                    name: "BLIP (Image Captioning)".to_string(),
                    description: Some("Local image captioning model".to_string()),
                    capabilities: raisin_ai::model_cache::ModelCapabilities {
                        chat: true,
                        streaming: false,
                        tools: false,
                        embeddings: false,
                        vision: true,
                    },
                    context_window: Some(512),
                    max_output_tokens: Some(256),
                    available: true,
                    metadata: None,
                },
                raisin_ai::model_cache::ModelInfo {
                    id: "clip".to_string(),
                    name: "CLIP (Image Embeddings)".to_string(),
                    description: Some("Local CLIP model for image embeddings".to_string()),
                    capabilities: raisin_ai::model_cache::ModelCapabilities {
                        chat: false,
                        streaming: false,
                        tools: false,
                        embeddings: true,
                        vision: true,
                    },
                    context_window: None,
                    max_output_tokens: None,
                    available: true,
                    metadata: None,
                },
            ])
        }
    }
}

/// Get models for a specific use case.
///
/// GET /api/tenants/{tenant_id}/ai/models/{use_case}
#[axum::debug_handler]
pub async fn list_models_by_use_case(
    Path((tenant_id, use_case)): Path<(String, AIUseCase)>,
    State(state): State<AppState>,
) -> Result<Json<ModelsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;
    let repo_impl = state.storage().tenant_ai_config_repository();

    match repo_impl.get_config(tenant_id).await {
        Ok(config) => {
            let mut models = Vec::new();

            for provider_config in config.providers {
                if !provider_config.enabled {
                    continue;
                }

                for model in provider_config.models {
                    if model.use_cases.contains(&use_case) {
                        models.push(ModelInfo {
                            model_id: model.model_id,
                            display_name: model.display_name,
                            provider: provider_config.provider,
                            use_cases: model.use_cases,
                            default_temperature: model.default_temperature,
                            default_max_tokens: model.default_max_tokens,
                        });
                    }
                }
            }

            Ok(Json(ModelsResponse { models }))
        }
        Err(raisin_ai::storage::StorageError::NotFound(_)) => {
            // Return empty list if not configured
            Ok(Json(ModelsResponse { models: Vec::new() }))
        }
        Err(e) => {
            tracing::error!("Failed to get AI config for {}: {}", tenant_id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ))
        }
    }
}
