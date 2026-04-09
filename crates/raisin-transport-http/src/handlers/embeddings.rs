//! HTTP handlers for tenant embedding configuration
//!
//! Provides REST API endpoints for:
//! - Getting tenant embedding configuration
//! - Setting/updating tenant embedding configuration
//! - Testing connection to embedding provider

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use raisin_embeddings::{
    config::{EmbeddingDistanceMetric, EmbeddingProvider, TenantEmbeddingConfig},
    crypto::ApiKeyEncryptor,
    storage::TenantEmbeddingConfigStore,
};

use crate::state::AppState;

/// Request body for setting tenant embedding config
///
/// Note: Per-node-type settings are now configured via NodeType schema
/// (indexable, index_types, and property-level index annotations)
#[derive(Debug, Deserialize)]
pub struct SetConfigRequest {
    pub enabled: bool,

    /// Reference to AI provider for embeddings (preferred over legacy provider field)
    #[serde(default)]
    pub ai_provider_ref: Option<String>,

    /// Reference to model within the provider
    #[serde(default)]
    pub ai_model_ref: Option<String>,

    pub provider: EmbeddingProvider,
    pub model: String,
    pub dimensions: usize,

    /// Plain-text API key (will be encrypted server-side)
    #[serde(default)]
    pub api_key_plain: Option<String>,

    pub include_name: bool,
    pub include_path: bool,
    pub max_embeddings_per_repo: Option<usize>,

    /// Chunking configuration
    #[serde(default)]
    pub chunking: Option<raisin_ai::config::ChunkingConfig>,

    /// Distance metric for vector similarity search (defaults to Cosine)
    #[serde(default)]
    pub distance_metric: Option<EmbeddingDistanceMetric>,
}

/// Response body for GET config (no API key exposed)
///
/// Note: Per-node-type settings are now configured via NodeType schema
#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub tenant_id: String,
    pub enabled: bool,

    /// Reference to AI provider for embeddings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_provider_ref: Option<String>,

    /// Reference to model within the provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_model_ref: Option<String>,

    pub provider: EmbeddingProvider,
    pub model: String,
    pub dimensions: usize,

    /// Indicates if API key is configured (don't expose the actual key)
    pub has_api_key: bool,

    pub include_name: bool,
    pub include_path: bool,
    pub max_embeddings_per_repo: Option<usize>,

    /// Chunking configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunking: Option<raisin_ai::config::ChunkingConfig>,
}

/// Response for test connection
#[derive(Debug, Serialize)]
pub struct TestConnectionResponse {
    pub success: bool,
    pub dimensions: Option<usize>,
    pub model: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Generic success response
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Get tenant embedding configuration
///
/// GET /api/tenants/{tenant_id}/embeddings/config
#[axum::debug_handler]
pub async fn get_tenant_embedding_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get repository from AppState
    let repo = state.storage().tenant_embedding_config_repository();

    // Fetch config
    let config = repo.get_config(&tenant_id).map_err(|e| {
        tracing::error!("Failed to get embedding config for {}: {}", tenant_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Storage error: {}", e),
            }),
        )
    })?;

    match config {
        Some(cfg) => {
            let response = ConfigResponse {
                tenant_id: cfg.tenant_id,
                enabled: cfg.enabled,
                ai_provider_ref: cfg.ai_provider_ref,
                ai_model_ref: cfg.ai_model_ref,
                provider: cfg.provider,
                model: cfg.model,
                dimensions: cfg.dimensions,
                has_api_key: cfg.api_key_encrypted.is_some(),
                include_name: cfg.include_name,
                include_path: cfg.include_path,
                max_embeddings_per_repo: cfg.max_embeddings_per_repo,
                chunking: cfg.chunking,
            };
            Ok(Json(response))
        }
        None => {
            // Return default config for tenant
            let default_config = TenantEmbeddingConfig::new(tenant_id.clone());
            let response = ConfigResponse {
                tenant_id,
                enabled: false,
                ai_provider_ref: None,
                ai_model_ref: None,
                provider: default_config.provider,
                model: default_config.model,
                dimensions: default_config.dimensions,
                has_api_key: false,
                include_name: default_config.include_name,
                include_path: default_config.include_path,
                max_embeddings_per_repo: default_config.max_embeddings_per_repo,
                chunking: None,
            };
            Ok(Json(response))
        }
    }
}

/// Set or update tenant embedding configuration
///
/// POST /api/tenants/{tenant_id}/embeddings/config
#[axum::debug_handler]
pub async fn set_tenant_embedding_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<SetConfigRequest>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo = state.storage().tenant_embedding_config_repository();

    // Build config from request
    let mut config = TenantEmbeddingConfig {
        tenant_id: tenant_id.clone(),
        enabled: req.enabled,
        ai_provider_ref: req.ai_provider_ref,
        ai_model_ref: req.ai_model_ref,
        provider: req.provider,
        model: req.model,
        dimensions: req.dimensions,
        api_key_encrypted: None, // Will set below if provided
        include_name: req.include_name,
        include_path: req.include_path,
        max_embeddings_per_repo: req.max_embeddings_per_repo,
        chunking: req.chunking,
        distance_metric: req.distance_metric.unwrap_or_default(),
    };

    // Encrypt API key if provided
    if let Some(plain_key) = req.api_key_plain {
        let master_key = state.get_master_key().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Master key not configured: {}", e),
                }),
            )
        })?;
        let encryptor = ApiKeyEncryptor::new(&master_key);

        let encrypted = encryptor.encrypt(&plain_key).map_err(|e| {
            tracing::error!("Failed to encrypt API key: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Encryption failed: {}", e),
                }),
            )
        })?;

        config.api_key_encrypted = Some(encrypted);
    } else {
        // Keep existing API key if not updating
        if let Ok(Some(existing)) = repo.get_config(&tenant_id) {
            config.api_key_encrypted = existing.api_key_encrypted;
        }
    }

    // Store config
    repo.set_config(&config).map_err(|e| {
        tracing::error!("Failed to store embedding config for {}: {}", tenant_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Storage error: {}", e),
            }),
        )
    })?;

    tracing::info!("Updated embedding config for tenant: {}", tenant_id);

    Ok(Json(SuccessResponse {
        success: true,
        message: format!("Embedding configuration saved for tenant {}", tenant_id),
    }))
}

/// Test connection to embedding provider
///
/// POST /api/tenants/{tenant_id}/embeddings/config/test
///
/// For Phase 1, this is a stub that validates the config exists and has an API key.
/// Phase 2 will implement actual provider testing.
#[axum::debug_handler]
pub async fn test_embedding_connection(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<TestConnectionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo = state.storage().tenant_embedding_config_repository();

    // Get config
    let config = repo
        .get_config(&tenant_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "No embedding configuration found".to_string(),
                }),
            )
        })?;

    // Ollama doesn't need an API key
    let is_ollama = matches!(config.provider, EmbeddingProvider::Ollama);

    if !is_ollama && config.api_key_encrypted.is_none() {
        return Ok(Json(TestConnectionResponse {
            success: false,
            dimensions: None,
            model: config.model,
            error: Some("No API key configured".to_string()),
        }));
    }

    tracing::info!(
        "Test connection for tenant {} (provider: {:?}, model: {})",
        tenant_id,
        config.provider,
        config.model
    );

    // Decrypt API key and test the provider
    let api_key = if is_ollama {
        String::new()
    } else {
        let master_key = state.get_master_key().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Master key error: {}", e),
                }),
            )
        })?;
        let encryptor = ApiKeyEncryptor::new(&master_key);
        encryptor
            .decrypt(config.api_key_encrypted.as_ref().unwrap())
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to decrypt API key: {}", e),
                    }),
                )
            })?
    };

    let provider = raisin_embeddings::create_provider(&config.provider, &api_key, &config.model)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid provider configuration: {}", e),
                }),
            )
        })?;

    match provider.test_connection().await {
        Ok(dims) => Ok(Json(TestConnectionResponse {
            success: true,
            dimensions: Some(dims),
            model: config.model,
            error: None,
        })),
        Err(e) => Ok(Json(TestConnectionResponse {
            success: false,
            dimensions: None,
            model: config.model,
            error: Some(format!("{}", e)),
        })),
    }
}
