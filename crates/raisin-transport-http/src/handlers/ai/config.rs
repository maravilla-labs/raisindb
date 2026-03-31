// SPDX-License-Identifier: BSL-1.1

//! AI configuration CRUD handlers.
//!
//! GET/PUT endpoints for tenant AI configuration and provider listing.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use raisin_ai::{
    config::{AIProviderConfig, TenantAIConfig},
    crypto::ApiKeyEncryptor,
    storage::TenantAIConfigStore,
};

use crate::state::AppState;

use super::types::{
    ConfigResponse, ErrorResponse, ProviderConfigResponse, ProviderSummary, ProvidersListResponse,
    SetConfigRequest, SuccessResponse,
};

/// Get full tenant AI configuration.
///
/// GET /api/tenants/{tenant_id}/ai/config
#[axum::debug_handler]
pub async fn get_ai_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;

    let repo_impl = state.storage().tenant_ai_config_repository();

    // Fetch config
    match repo_impl.get_config(tenant_id).await {
        Ok(config) => {
            let providers = config
                .providers
                .into_iter()
                .map(|p| ProviderConfigResponse {
                    provider: p.provider,
                    has_api_key: p.api_key_encrypted.is_some(),
                    api_endpoint: p.api_endpoint,
                    enabled: p.enabled,
                    models: p.models,
                })
                .collect();

            Ok(Json(ConfigResponse {
                tenant_id: config.tenant_id,
                providers,
                embedding_settings: config.embedding_settings,
            }))
        }
        Err(raisin_ai::storage::StorageError::NotFound(_)) => {
            // Return empty config for tenant if not found
            Ok(Json(ConfigResponse {
                tenant_id: tenant_id.to_string(),
                providers: Vec::new(),
                embedding_settings: None,
            }))
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

/// Set or update full tenant AI configuration.
///
/// PUT /api/tenants/{tenant_id}/ai/config
#[axum::debug_handler]
pub async fn set_ai_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<SetConfigRequest>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;
    let repo_impl = state.storage().tenant_ai_config_repository();

    // Build config from request
    let mut config = TenantAIConfig {
        tenant_id: tenant_id.to_string(),
        providers: Vec::new(),
        embedding_settings: req.embedding_settings,
        processing_defaults: None, // TODO: Add to SetConfigRequest if needed
    };

    let master_key = state.get_master_key().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Master key not configured: {}", e),
            }),
        )
    })?;
    let encryptor = ApiKeyEncryptor::new(&master_key);

    // Process each provider
    for provider_req in req.providers {
        let mut provider_config = AIProviderConfig {
            provider: provider_req.provider,
            api_key_encrypted: None,
            api_endpoint: provider_req.api_endpoint,
            enabled: provider_req.enabled,
            models: provider_req.models,
        };

        // Encrypt API key if provided
        if let Some(plain_key) = provider_req.api_key_plain {
            let encrypted = encryptor.encrypt(&plain_key).map_err(|e| {
                tracing::error!("Failed to encrypt API key: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Encryption failed: {}", e),
                    }),
                )
            })?;

            provider_config.api_key_encrypted = Some(encrypted);
        } else {
            // Keep existing API key if not updating
            if let Ok(existing) = repo_impl.get_config(tenant_id).await {
                if let Some(existing_provider) = existing
                    .providers
                    .iter()
                    .find(|p| p.provider == provider_req.provider)
                {
                    provider_config.api_key_encrypted = existing_provider.api_key_encrypted.clone();
                }
            }
        }

        config.providers.push(provider_config);
    }

    // Store config
    repo_impl.set_config(&config).await.map_err(|e| {
        tracing::error!("Failed to store AI config for {}: {}", tenant_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Storage error: {}", e),
            }),
        )
    })?;

    tracing::info!(
        "Updated AI config for tenant: {} ({} providers)",
        tenant_id,
        config.providers.len()
    );

    Ok(Json(SuccessResponse {
        success: true,
        message: format!("AI configuration saved for tenant {}", tenant_id),
    }))
}

/// List all configured providers.
///
/// GET /api/tenants/{tenant_id}/ai/providers
#[axum::debug_handler]
pub async fn list_providers(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ProvidersListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;
    let repo_impl = state.storage().tenant_ai_config_repository();

    match repo_impl.get_config(tenant_id).await {
        Ok(config) => {
            let providers = config
                .providers
                .into_iter()
                .map(|p| ProviderSummary {
                    provider: p.provider,
                    enabled: p.enabled,
                    has_api_key: p.api_key_encrypted.is_some(),
                    model_count: p.models.len(),
                })
                .collect();

            Ok(Json(ProvidersListResponse { providers }))
        }
        Err(raisin_ai::storage::StorageError::NotFound(_)) => {
            // Return empty list if not configured
            Ok(Json(ProvidersListResponse {
                providers: Vec::new(),
            }))
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
