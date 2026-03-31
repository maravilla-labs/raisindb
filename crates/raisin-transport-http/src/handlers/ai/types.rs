// SPDX-License-Identifier: BSL-1.1

//! Request and response types for AI configuration HTTP endpoints.

use raisin_ai::{
    config::{AIModelConfig, AIProvider, AIUseCase, EmbeddingSettings},
    DownloadStatus, HFModelInfo,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// Config types
// ============================================================================

/// Request body for setting full tenant AI config.
#[derive(Debug, Deserialize)]
pub struct SetConfigRequest {
    /// List of provider configurations
    pub providers: Vec<ProviderConfigRequest>,

    /// Embedding-specific settings
    #[serde(default)]
    pub embedding_settings: Option<EmbeddingSettings>,
}

/// Provider configuration in request (with plain API key).
#[derive(Debug, Deserialize)]
pub struct ProviderConfigRequest {
    pub provider: AIProvider,

    /// Plain-text API key (will be encrypted server-side)
    #[serde(default)]
    pub api_key_plain: Option<String>,

    #[serde(default)]
    pub api_endpoint: Option<String>,

    pub enabled: bool,

    #[serde(default)]
    pub models: Vec<AIModelConfig>,
}

/// Response body for GET config (no API keys exposed).
#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub tenant_id: String,
    pub providers: Vec<ProviderConfigResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_settings: Option<EmbeddingSettings>,
}

/// Provider configuration in response (API key presence only).
#[derive(Debug, Serialize)]
pub struct ProviderConfigResponse {
    pub provider: AIProvider,

    /// Indicates if API key is configured (don't expose the actual key)
    pub has_api_key: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,

    pub enabled: bool,
    pub models: Vec<AIModelConfig>,
}

// ============================================================================
// Provider listing types
// ============================================================================

/// Response for provider listing.
#[derive(Debug, Serialize)]
pub struct ProvidersListResponse {
    pub providers: Vec<ProviderSummary>,
}

/// Summary of a configured provider.
#[derive(Debug, Serialize)]
pub struct ProviderSummary {
    pub provider: AIProvider,
    pub enabled: bool,
    pub has_api_key: bool,
    pub model_count: usize,
}

// ============================================================================
// Connection testing types
// ============================================================================

/// Response for test connection.
#[derive(Debug, Serialize)]
pub struct TestConnectionResponse {
    pub success: bool,
    pub provider: AIProvider,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ============================================================================
// Model discovery types
// ============================================================================

/// Response for model discovery.
#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

/// Information about a discovered model.
#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub model_id: String,
    pub display_name: String,
    pub provider: AIProvider,
    pub use_cases: Vec<AIUseCase>,
    pub default_temperature: f32,
    pub default_max_tokens: u32,
}

/// Generic success response.
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Query parameters for list models endpoint.
#[derive(Debug, Deserialize)]
pub struct ListModelsQuery {
    /// Filter by specific provider
    #[serde(default)]
    pub provider: Option<AIProvider>,

    /// If true, fetch models from provider APIs instead of returning cached
    #[serde(default)]
    pub refresh: bool,
}

// ============================================================================
// Model capabilities types
// ============================================================================

/// Response for model capabilities query.
#[derive(Debug, Serialize)]
pub struct ModelCapabilitiesResponse {
    pub model_id: String,
    pub provider: AIProvider,
    pub capabilities: CapabilitiesInfo,
}

/// Detailed capabilities information.
#[derive(Debug, Serialize)]
pub struct CapabilitiesInfo {
    pub chat: bool,
    pub embeddings: bool,
    pub vision: bool,
    pub tools: bool,
    pub streaming: bool,
}

// ============================================================================
// HuggingFace model types
// ============================================================================

/// Response for HuggingFace model info.
#[derive(Debug, Serialize)]
pub struct HuggingFaceModelResponse {
    pub model_id: String,
    pub display_name: String,
    pub model_type: String,
    pub capabilities: Vec<String>,
    pub estimated_size_bytes: Option<u64>,
    pub actual_size_bytes: Option<u64>,
    pub status: HuggingFaceDownloadStatusResponse,
    pub description: Option<String>,
    pub model_url: String,
    pub size_display: String,
}

/// Download status for HuggingFace model.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HuggingFaceDownloadStatusResponse {
    NotDownloaded,
    Downloading {
        progress: f32,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Ready,
    Failed {
        error: String,
    },
}

impl From<DownloadStatus> for HuggingFaceDownloadStatusResponse {
    fn from(status: DownloadStatus) -> Self {
        match status {
            DownloadStatus::NotDownloaded => Self::NotDownloaded,
            DownloadStatus::Downloading {
                progress,
                downloaded_bytes,
                total_bytes,
            } => Self::Downloading {
                progress,
                downloaded_bytes,
                total_bytes,
            },
            DownloadStatus::Ready => Self::Ready,
            DownloadStatus::Failed { error } => Self::Failed { error },
        }
    }
}

impl From<HFModelInfo> for HuggingFaceModelResponse {
    fn from(model: HFModelInfo) -> Self {
        let size_display = model.size_display();
        Self {
            model_id: model.model_id,
            display_name: model.display_name,
            model_type: model.model_type.to_string(),
            capabilities: model
                .capabilities
                .iter()
                .map(|c| format!("{:?}", c))
                .collect(),
            estimated_size_bytes: model.estimated_size_bytes,
            actual_size_bytes: model.actual_size_bytes,
            status: model.status.into(),
            description: model.description,
            model_url: model.model_url,
            size_display,
        }
    }
}

/// Response for list of HuggingFace models.
#[derive(Debug, Serialize)]
pub struct HuggingFaceModelsListResponse {
    pub models: Vec<HuggingFaceModelResponse>,
    pub total_disk_usage: String,
}

/// Response for model download initiation.
#[derive(Debug, Serialize)]
pub struct HuggingFaceModelDownloadResponse {
    pub model_id: String,
    pub job_id: String,
    pub message: String,
}

/// Response for model deletion.
#[derive(Debug, Serialize)]
pub struct HuggingFaceModelDeleteResponse {
    pub model_id: String,
    pub success: bool,
    pub message: String,
}

// ============================================================================
// Local captioning model types
// ============================================================================

/// Response for local captioning model info.
#[derive(Debug, Serialize)]
pub struct LocalCaptionModelResponse {
    /// Model ID (e.g., "Salesforce/blip-image-captioning-large")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Approximate model size in MB
    pub size_mb: u32,
    /// Whether this model is currently supported
    pub supported: bool,
    /// Brief description
    pub description: String,
}

/// Response for listing local captioning models.
#[derive(Debug, Serialize)]
pub struct LocalCaptionModelsResponse {
    pub models: Vec<LocalCaptionModelResponse>,
    pub default_model: String,
}
