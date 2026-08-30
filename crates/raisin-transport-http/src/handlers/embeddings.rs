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
    config::{
        EmbeddingDistanceMetric, EmbeddingProvider, EmbeddingQuantization, TenantEmbeddingConfig,
    },
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

    /// Base URL for self-hosted providers (e.g., Ollama remote instance)
    #[serde(default)]
    pub base_url: Option<String>,

    /// Scalar precision the HNSW index stores vectors at (`F32` | `F16` | `Int8`).
    ///
    /// The admin console has SENT this field since it shipped
    /// (`TenantEmbeddingSettings.tsx` renders the `<select>` and puts the value
    /// in its save payload). There was nowhere for it to land, so serde dropped
    /// it and every index was F32 — a control that looked live and did nothing.
    /// Now it reaches `TenantEmbeddingConfig` and, through the engine's
    /// `IndexSpecResolver`, the usearch `ScalarKind` an index is built with.
    ///
    /// Takes effect on the NEXT index build: the scalar kind is baked into the
    /// graph and persisted in its `.hnsw.meta` sidecar, so an existing index
    /// keeps the precision it was written with until `REBUILD VECTOR INDEX`.
    #[serde(default)]
    pub quantization: Option<EmbeddingQuantization>,
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

    /// Base URL for self-hosted providers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Scalar precision new HNSW indexes are built at.
    ///
    /// Echoed back so the console's `<select>` shows what was actually stored
    /// rather than what it last sent — the two used to differ silently.
    pub quantization: EmbeddingQuantization,
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
                base_url: cfg.base_url,
                quantization: cfg.quantization,
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
                base_url: None,
                quantization: default_config.quantization,
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
        default_max_distance: None,
        distance_metric: req.distance_metric.unwrap_or_default(),
        base_url: req.base_url,
        quantization: req.quantization.unwrap_or_default(),
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
        // Keep the existing API key when the request does not supply one.
        //
        // A failed read must NOT fall through to the write. `if let Ok(Some(_))` treated
        // "the read blew up" and "there is no config yet" as the same case, so a
        // deserialization error or a backend hiccup left `api_key_encrypted` at `None` and
        // the store below happily persisted that — destroying the only copy of a credential
        // nothing can restore, during a request that was not even trying to change it.
        // Absent is fine; broken is a 500 with no write.
        match repo.get_config(&tenant_id) {
            Ok(Some(existing)) => config.api_key_encrypted = existing.api_key_encrypted,
            Ok(None) => {}
            Err(e) => {
                tracing::error!(
                    "Refusing to write embedding config for {}: the existing one could not \
                     be read, and writing over it would drop the stored API key: {}",
                    tenant_id,
                    e
                );
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Could not read the existing configuration: {}", e),
                    }),
                ));
            }
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

    // Whether a key is required is asked ONCE, inside the shared resolver, of
    // the provider variant itself. This endpoint used to hard-code
    // `matches!(config.provider, Ollama)` while the embedding job handler
    // demanded a key unconditionally - so "Test connection" returned a green
    // 768-dimension success for a config under which every job then failed
    // with "No API key configured for embeddings", and nothing surfaced it.
    //
    // It also used `create_provider_with_url`, which drops `dimensions`: a
    // model outside the vendor's built-in name table tested as an "invalid
    // provider configuration" even when the endpoint was perfectly reachable.
    let rocksdb = state.rocksdb_storage().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "RocksDB storage required for embedding configuration".to_string(),
            }),
        )
    })?;

    let master_key = state.get_master_key().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Master key error: {}", e),
            }),
        )
    })?;

    tracing::info!(
        "Test connection for tenant {} (provider: {:?}, model: {})",
        tenant_id,
        config.provider,
        config.model
    );

    // A resolution failure is the answer, not a 5xx: it is exactly what the
    // job would hit, and reporting it in the same shape as a transport failure
    // is what makes this endpoint honest about the job's fate.
    // `resolve_settings` first, so the response can name the model the job will
    // actually request. Reporting `config.model` here is a lie under a unified
    // `ai_provider_ref`, where the legacy field is stale by construction: this
    // endpoint would report "text-embedding-3-small" for a test it had just run
    // against nomic-embed-text.
    let resolved = match raisin_rocksdb::embedding_provider::resolve_settings(
        rocksdb,
        &tenant_id,
        &config,
        &master_key,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(TestConnectionResponse {
                success: false,
                dimensions: None,
                model: config.model,
                error: Some(format!("{}", e)),
            }))
        }
    };
    let model = resolved.model.clone();

    let provider = match resolved.build() {
        Ok(p) => p,
        Err(e) => {
            return Ok(Json(TestConnectionResponse {
                success: false,
                dimensions: None,
                model,
                error: Some(format!("{}", e)),
            }))
        }
    };

    match provider.test_connection().await {
        Ok(dims) => Ok(Json(TestConnectionResponse {
            success: true,
            dimensions: Some(dims),
            model,
            error: None,
        })),
        Err(e) => Ok(Json(TestConnectionResponse {
            success: false,
            dimensions: None,
            model,
            error: Some(format!("{}", e)),
        })),
    }
}
