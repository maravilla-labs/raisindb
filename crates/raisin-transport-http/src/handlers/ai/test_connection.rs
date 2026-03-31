// SPDX-License-Identifier: BSL-1.1

//! Provider connection testing handler.
//!
//! Tests a live connection to a configured AI provider by calling its
//! model-listing API. Updates the provider's model list on success.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use raisin_ai::{
    config::{AIModelConfig, AIProvider, AIUseCase},
    crypto::ApiKeyEncryptor,
    model_cache::ModelCapabilities,
    provider::AIProviderTrait,
    providers::{
        AnthropicProvider, AzureOpenAIProvider, BedrockProvider, GeminiProvider, GroqProvider,
        OllamaProvider, OpenAIProvider, OpenRouterProvider,
    },
    storage::TenantAIConfigStore,
};

use crate::state::AppState;

use super::types::{ErrorResponse, TestConnectionResponse};

/// Test connection to a specific provider.
///
/// POST /api/tenants/{tenant_id}/ai/providers/{provider}/test
///
/// Actually tests the connection by calling the provider's API to fetch models.
#[axum::debug_handler]
pub async fn test_provider_connection(
    Path((tenant_id, provider)): Path<(String, AIProvider)>,
    State(state): State<AppState>,
) -> Result<Json<TestConnectionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id_str = tenant_id.clone();
    let repo_impl = state.storage().tenant_ai_config_repository();

    // Get config
    let config = repo_impl
        .get_config(&tenant_id)
        .await
        .map_err(|e| match e {
            raisin_ai::storage::StorageError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "No AI configuration found".to_string(),
                }),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ),
        })?;

    // Find the specific provider
    let provider_config = config
        .providers
        .iter()
        .find(|p| p.provider == provider)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Provider {:?} not configured", provider),
                }),
            )
        })?;

    // Check if API key is configured (if required)
    if provider.requires_api_key() && provider_config.api_key_encrypted.is_none() {
        return Ok(Json(TestConnectionResponse {
            success: false,
            provider,
            message: None,
            error: Some("No API key configured".to_string()),
        }));
    }

    // Decrypt API key if present
    let api_key = if let Some(encrypted) = &provider_config.api_key_encrypted {
        let master_key = state.get_master_key().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Master key not configured: {}", e),
                }),
            )
        })?;
        let encryptor = ApiKeyEncryptor::new(&master_key);
        Some(encryptor.decrypt(encrypted).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to decrypt API key: {}", e),
                }),
            )
        })?)
    } else {
        None
    };

    let endpoint = provider_config.api_endpoint.as_deref();

    tracing::info!(
        "Testing connection for tenant {} provider {:?}",
        tenant_id_str,
        provider
    );

    // Create provider instance and fetch models
    let models_result = call_provider_list_models(provider, api_key.as_deref(), endpoint).await;

    match models_result {
        Ok(fetched_models) => {
            // Convert ModelInfo to AIModelConfig
            let model_configs: Vec<AIModelConfig> = fetched_models
                .into_iter()
                .map(|m| AIModelConfig {
                    model_id: m.id,
                    display_name: m.name,
                    use_cases: convert_capabilities_to_use_cases(&m.capabilities),
                    default_temperature: 0.7,
                    default_max_tokens: 4096,
                    is_default: false,
                    metadata: m.metadata,
                })
                .collect();

            let model_count = model_configs.len();

            // Update provider config with fetched models
            let mut updated_config = config.clone();
            if let Some(pc) = updated_config
                .providers
                .iter_mut()
                .find(|p| p.provider == provider)
            {
                pc.models = model_configs;
            }

            // Save updated config
            if let Err(e) = repo_impl.set_config(&updated_config).await {
                tracing::warn!("Failed to save updated models: {}", e);
            }

            tracing::info!(
                "Successfully connected to {:?}, found {} models",
                provider,
                model_count
            );

            Ok(Json(TestConnectionResponse {
                success: true,
                provider,
                message: Some(format!(
                    "Connected successfully. Found {} models.",
                    model_count
                )),
                error: None,
            }))
        }
        Err(e) => {
            tracing::warn!("Failed to connect to {:?}: {}", provider, e);
            Ok(Json(TestConnectionResponse {
                success: false,
                provider,
                message: None,
                error: Some(format!("Failed to connect: {}", e)),
            }))
        }
    }
}

/// Call a provider's list_available_models (or test_connection for Ollama).
async fn call_provider_list_models(
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
            p.list_available_models().await.map_err(|e| e.to_string())
        }
        AIProvider::Anthropic => {
            let p = AnthropicProvider::new(api_key.unwrap_or_default());
            p.list_available_models().await.map_err(|e| e.to_string())
        }
        AIProvider::Google => {
            let p = match endpoint {
                Some(url) => GeminiProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => GeminiProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models().await.map_err(|e| e.to_string())
        }
        AIProvider::AzureOpenAI => {
            if let Some(azure_endpoint) = endpoint {
                let p = AzureOpenAIProvider::new(api_key.unwrap_or_default(), azure_endpoint);
                p.list_available_models().await.map_err(|e| e.to_string())
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
            p.test_connection().await.map_err(|e| e.to_string())
        }
        AIProvider::Groq => {
            let p = match endpoint {
                Some(url) => GroqProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => GroqProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models().await.map_err(|e| e.to_string())
        }
        AIProvider::OpenRouter => {
            let p = match endpoint {
                Some(url) => OpenRouterProvider::with_base_url(api_key.unwrap_or_default(), url),
                None => OpenRouterProvider::new(api_key.unwrap_or_default()),
            };
            p.list_available_models().await.map_err(|e| e.to_string())
        }
        AIProvider::Bedrock => {
            if let Some(region) = endpoint {
                let key = api_key.unwrap_or_default();
                let parts: Vec<&str> = key.splitn(2, ':').collect();
                if parts.len() == 2 {
                    let p = BedrockProvider::new(region, parts[0], parts[1]);
                    p.list_available_models().await.map_err(|e| e.to_string())
                } else {
                    Err(
                        "AWS Bedrock api_key must be in format 'access_key_id:secret_access_key'"
                            .to_string(),
                    )
                }
            } else {
                Err("AWS Bedrock requires a region in the endpoint field".to_string())
            }
        }
        AIProvider::Custom => Ok(vec![]),
        AIProvider::Local => Ok(vec![]),
    }
}

/// Convert model capabilities to AI use cases.
fn convert_capabilities_to_use_cases(caps: &ModelCapabilities) -> Vec<AIUseCase> {
    let mut use_cases = vec![];
    if caps.chat {
        use_cases.push(AIUseCase::Chat);
    }
    if caps.embeddings {
        use_cases.push(AIUseCase::Embedding);
    }
    if caps.tools {
        use_cases.push(AIUseCase::Agent);
    }
    use_cases
}
