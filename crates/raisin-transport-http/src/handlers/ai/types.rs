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

    /// Embedding-specific settings.
    ///
    /// Absent — which every pre-slug client's payload is — keeps what is stored.
    /// There is no way to clear it through this endpoint, on purpose: it carries
    /// `dimensions`, which is hashed into the tenant's embedder identity and
    /// therefore into the key of every vector already written. Nulling it orphans
    /// the lot, so an accidental `null` must not be able to do it either.
    #[serde(default)]
    pub embedding_settings: Option<EmbeddingSettings>,

    /// Tenant-wide asset processing defaults (OCR languages, confidence floor).
    ///
    /// Absent keeps what is stored, exactly like `embedding_settings` — a client
    /// written before this field existed must not clear it by omission.
    #[serde(default)]
    pub processing_defaults: Option<raisin_ai::config::ProcessingDefaults>,
}

/// A field of a MERGE payload: absent, explicitly cleared, or set.
///
/// `None` is "the client did not mention this field" and means *keep what is
/// stored*; `Some(None)` is an explicit `null` and means *clear it*;
/// `Some(Some(v))` sets it. A plain `Option` cannot tell the first two apart,
/// and under merge semantics that difference is the whole game — the admin
/// console has never sent `display_name` or `icon_url` at all, so reading its
/// silence as `null` erased the gateway's name and logo on the first save.
///
/// The distinction only survives because of [`deserialize_present`]: serde's
/// stock `Option` impl maps a JSON `null` to `None`, collapsing the two states
/// again.
pub type Patch<T> = Option<Option<T>>;

/// Deserializes a field as *present*, so `null` lands as `Some(None)` rather
/// than being folded into the missing-field default of `None`.
fn deserialize_present<'de, T, D>(deserializer: D) -> Result<Patch<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Provider configuration in request (with plain API key).
#[derive(Debug, Deserialize)]
pub struct ProviderConfigRequest {
    /// Per-tenant identifier, chosen by the caller and immutable once created.
    /// Doubles as the model-id prefix (`<slug>:<model>`).
    ///
    /// Optional on the wire, and absent means "the kind's serde name" — the same
    /// default stored entries get. That is what lets a client written before slugs
    /// existed keep PUTting the config it just GET'd, instead of having every entry
    /// read as a delete plus a create.
    #[serde(default)]
    pub slug: Option<String>,

    /// The provider KIND — which wire protocol to speak. Keeps the field name
    /// `provider`, which is what every existing client already sends.
    pub provider: AIProvider,

    /// Human-readable name shown in the UI (defaults to the slug when absent).
    ///
    /// Three-state, like every optional field below it: absent keeps whatever is
    /// stored, an explicit `null` clears it, a value sets it. See [`Patch`].
    #[serde(default, deserialize_with = "deserialize_present")]
    pub display_name: Patch<String>,

    /// Optional logo URL shown next to the display name.
    #[serde(default, deserialize_with = "deserialize_present")]
    pub icon_url: Patch<String>,

    /// Plain-text API key (will be encrypted server-side).
    ///
    /// Omitting it preserves the stored key — that is how a client saves an edit to
    /// any other field without having to hold the secret. Deliberately NOT a
    /// [`Patch`]: there is no "clear the key" through this field, because a client
    /// that JSON-serializes its whole form with empty fields as `null` would then
    /// destroy the one value nothing else can restore. Dropping an entry's
    /// credential is done by deleting the entry.
    #[serde(default)]
    pub api_key_plain: Option<String>,

    /// The entry's endpoint. Three-state; see [`Patch`].
    #[serde(default, deserialize_with = "deserialize_present")]
    pub api_endpoint: Patch<String>,

    pub enabled: bool,

    /// The entry's registered models. Three-state, for the same reason the descriptive
    /// fields are: `#[serde(default)]` on a bare `Vec` makes "the client did not mention
    /// models" indistinguishable from "the client wants no models", so a partial update
    /// silently blanked a synced catalogue. Absent keeps what is stored; `null` and `[]`
    /// both clear it, which is the one case a caller can still state explicitly.
    #[serde(default, deserialize_with = "deserialize_present")]
    pub models: Patch<Vec<AIModelConfig>>,
}

impl ProviderConfigRequest {
    /// The slug this entry is addressed by, with the legacy default applied.
    pub fn slug(&self) -> &str {
        self.slug
            .as_deref()
            .unwrap_or_else(|| self.provider.serde_name())
    }
}

/// Response body for GET config (no API keys exposed).
#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub tenant_id: String,
    pub providers: Vec<ProviderConfigResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_settings: Option<EmbeddingSettings>,
    /// Tenant-wide asset processing defaults. Was never returned before, so the
    /// console could not render a value it was perfectly able to store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_defaults: Option<raisin_ai::config::ProcessingDefaults>,
}

/// Provider configuration in response (API key presence only).
#[derive(Debug, Serialize)]
pub struct ProviderConfigResponse {
    /// Per-tenant identifier; also the model-id prefix.
    pub slug: String,

    /// The provider kind.
    pub provider: AIProvider,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,

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
    /// Per-tenant identifier; also the model-id prefix.
    pub slug: String,
    /// The provider kind.
    pub provider: AIProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// The entry's endpoint, when it has one. Not a secret, and it is what tells two
    /// same-kind entries apart at a glance — a listing of three `custom` gateways is
    /// otherwise three identical rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,
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
    /// The slug that was tested (the `{provider}` path segment).
    pub slug: String,
    /// The kind behind that slug.
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
    /// The provider kind. Left as the kind rather than swapped for the slug: this
    /// field is what clients switch on to pick an icon or a protocol hint.
    pub provider: AIProvider,
    /// The slug of the entry serving this model — the prefix that addresses it as
    /// `<provider_slug>:<model_id>`.
    pub provider_slug: String,
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
    /// Filter by a specific provider SLUG (e.g. `openai`, or `marvel` for a
    /// self-named gateway). Legacy callers passing a kind name still match,
    /// because pre-slug entries are slugged after their kind's serde name.
    #[serde(default)]
    pub provider: Option<String>,

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
    /// The provider kind the capabilities were derived from.
    pub provider: AIProvider,
    /// The slug that was queried. The per-kind helpers below build the rest of this
    /// struct and have no notion of slugs, so the handler stamps this field once on
    /// the way out — see `get_model_capabilities`.
    pub provider_slug: String,
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
