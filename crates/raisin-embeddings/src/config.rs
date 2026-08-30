//! Configuration models for tenant-level embedding settings.

use serde::{Deserialize, Serialize};

/// Tenant-level embedding configuration.
///
/// This structure stores all embedding-related settings for a tenant, including:
/// - Provider and model configuration
/// - Encrypted API keys
/// - Content generation defaults (name, path)
/// - Usage limits
///
/// Note: Per-node-type configuration is now handled via NodeType schema fields
/// (indexable, index_types, and property-level index annotations)
///
/// # Provider Reference (Unified AI Providers)
///
/// The preferred way to configure embedding providers is via `ai_provider_ref` and
/// `ai_model_ref`, which reference a provider/model configured in `TenantAIConfig`.
/// This allows using the same API keys and endpoints for both chat and embeddings.
///
/// If these refs are not set, the legacy `provider`, `model`, and `api_key_encrypted`
/// fields are used for backward compatibility.
///
/// # Backward Compatibility
///
/// This struct previously had a `node_type_settings` field that has been removed.
/// The `deny_unknown_fields` attribute has been removed to allow old MessagePack data
/// containing this field to deserialize successfully (the field will be ignored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantEmbeddingConfig {
    /// The tenant identifier
    pub tenant_id: String,

    /// Whether embeddings are enabled for this tenant
    pub enabled: bool,

    /// Reference to AI provider for embeddings, by the provider's per-tenant SLUG
    /// (e.g. "openai", or "marvel" for a self-named gateway).
    /// If set, uses that provider from TenantAIConfig instead of legacy fields.
    ///
    /// Nothing enforces referential integrity here, which is exactly why slugs are
    /// immutable: a rename would leave this dangling, and the job would fail at
    /// embedding time rather than at config time. Refs written before slugs existed
    /// still resolve, because pre-slug entries are slugged after their kind's serde
    /// name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_provider_ref: Option<String>,

    /// Reference to model within the provider (e.g., "text-embedding-3-small").
    /// If set, uses the model from TenantAIConfig provider configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_model_ref: Option<String>,

    /// The embedding provider to use (LEGACY - use ai_provider_ref instead)
    pub provider: EmbeddingProvider,

    /// The model name/identifier (LEGACY - use ai_model_ref instead)
    pub model: String,

    /// Vector dimensionality (e.g., 1536 for text-embedding-3-small)
    pub dimensions: usize,

    /// Encrypted API key (stored, never returned to client)
    /// Uses AES-256-GCM encryption
    /// (LEGACY - when using ai_provider_ref, the key comes from TenantAIConfig)
    #[serde(default)]
    pub api_key_encrypted: Option<Vec<u8>>,

    /// Whether to include node name in embedding content
    pub include_name: bool,

    /// Whether to include node path in embedding content
    pub include_path: bool,

    /// Maximum number of embeddings allowed per repository
    /// None means unlimited (within tenant limits)
    pub max_embeddings_per_repo: Option<usize>,

    /// Chunking configuration for splitting large text into smaller chunks.
    /// If None, chunking is disabled (single embedding per node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunking: Option<raisin_ai::config::ChunkingConfig>,

    /// Default max distance threshold for vector search.
    /// Results beyond this threshold are filtered out.
    /// If None, uses the engine default (0.6 for cosine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_max_distance: Option<f32>,

    /// Distance metric for vector similarity search.
    /// Defaults to Cosine for backward compatibility.
    /// Changing this requires a full index rebuild.
    #[serde(default)]
    pub distance_metric: EmbeddingDistanceMetric,

    /// Optional base URL for self-hosted or remote provider endpoints.
    /// Used by Ollama (defaults to http://localhost:11434 if not set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Scalar precision the HNSW index stores vectors at.
    ///
    /// The admin console has offered this control since it shipped
    /// (`packages/admin-console/src/api/ai.ts` defines the enum, and
    /// `TenantEmbeddingSettings.tsx` renders a live `<select>` and sends the
    /// value in its save payload) — and until this field existed the Rust side
    /// had nowhere to put it, so it was dropped on the way in and every index
    /// was F32 regardless. A shipped control that silently does nothing is worse
    /// than no control.
    ///
    /// Changing it requires `REBUILD VECTOR INDEX`: the scalar kind is baked
    /// into the usearch graph at construction and is persisted in the `.hnsw.meta`
    /// sidecar, so an existing index keeps the precision it was built with.
    #[serde(default)]
    pub quantization: EmbeddingQuantization,
}

/// Scalar precision an HNSW index stores its vectors at.
///
/// Variant names are the wire format the admin console already sends
/// (`'F32' | 'F16' | 'Int8'`), so the two cannot disagree.
///
/// `Int8` is a configuration change only: usearch casts the caller's `&[f32]`
/// into the index's scalar kind on insert and `scalar_words() == dimensions` for
/// f32/f16/i8, so no signature changes. The f32→i8 cast L2-normalises and scales
/// to ±127 and assumes a dot-product-like metric — which holds here because
/// vectors are normalised on the way in and the metric is pinned in the same
/// `IndexSpec` the quantization comes from.
///
/// There is deliberately no `B1` (binary / Hamming) variant: `scalar_words()`
/// for b1 is `dimensions / 8`, so an `&[f32]` is rejected at the usearch binding
/// boundary. It needs `add_b1x8` / `search_b1x8` / `filtered_search_b1x8`,
/// caller-side bit packing, and a distance scale under which
/// `DEFAULT_MAX_DISTANCE = 0.6` means something. Deferred, not forgotten.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EmbeddingQuantization {
    /// 32-bit float. Full precision; ~4 bytes per dimension per vector.
    #[default]
    F32,
    /// 16-bit float. ~50% payload reduction.
    F16,
    /// 8-bit signed integer. ~75% payload reduction — a 1024-wide, 50k-vector
    /// text index goes from ~222 MB to ~68 MB against the engine's shared
    /// 512 MB budget.
    Int8,
}

/// Supported embedding providers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EmbeddingProvider {
    /// OpenAI embedding models (text-embedding-3-small, text-embedding-3-large, etc.)
    OpenAI,

    /// Voyage embeddings via Anthropic
    Claude,

    /// Local models via Ollama
    Ollama,

    /// HuggingFace local inference (requires 'candle' feature)
    HuggingFace,
}

impl EmbeddingProvider {
    /// Whether this provider requires an API key, given where it points.
    ///
    /// Asked in exactly ONE place — [`crate::resolve::resolve_settings`] — so a
    /// keyless provider is exempt everywhere or nowhere. Before this, the HTTP
    /// "test connection" endpoint hard-coded `matches!(provider, Ollama)` while
    /// the embedding job handler demanded a key unconditionally: the test went
    /// green with 768 dimensions for a config under which every job then failed
    /// with *No API key configured for embeddings*, and nothing surfaced it.
    ///
    /// `base_url` is part of the question, not decoration. `OpenAI` here means
    /// "speaks OpenAI's `/embeddings` shape", and a self-hosted server that
    /// speaks it (vLLM, LocalAI, a gateway) authenticates however it likes —
    /// usually not at all. A key is therefore only *required* when the request
    /// is actually going to the vendor's own host. Demanding one regardless is
    /// what forced `SET API_KEY='unused-but-required'` into self-hosted configs.
    ///
    /// Mirrors [`raisin_ai::config::AIProvider::requires_api_key`], which is
    /// what the unified-provider branch asks of its own provider kinds.
    pub fn requires_api_key(&self, base_url: Option<&str>) -> bool {
        match self {
            // Local: nothing to authenticate against, ever.
            EmbeddingProvider::Ollama | EmbeddingProvider::HuggingFace => false,
            // Vendor-hosted unless pointed somewhere else.
            EmbeddingProvider::OpenAI | EmbeddingProvider::Claude => base_url.is_none(),
        }
    }
}

/// Distance metric for embedding similarity search.
///
/// This is stored in the tenant embedding config and determines
/// how the HNSW index computes distances between vectors.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EmbeddingDistanceMetric {
    /// Cosine distance (default, best for most embedding models)
    #[default]
    Cosine,
    /// Euclidean (L2) distance
    L2,
    /// Inner product distance
    InnerProduct,
    /// Hamming distance (for binary vectors)
    Hamming,
}

impl TenantEmbeddingConfig {
    /// Create a new configuration with default settings.
    ///
    /// Defaults:
    /// - Embeddings disabled
    /// - OpenAI provider with text-embedding-3-small model
    /// - 1536 dimensions
    /// - Include name and path in content
    ///
    /// Note: Per-node-type settings are now configured via NodeType schema
    pub fn new(tenant_id: String) -> Self {
        Self {
            tenant_id,
            enabled: false,
            ai_provider_ref: None,
            ai_model_ref: None,
            provider: EmbeddingProvider::OpenAI,
            model: "text-embedding-3-small".to_string(),
            dimensions: 1536,
            api_key_encrypted: None,
            include_name: true,
            include_path: true,
            max_embeddings_per_repo: None,
            chunking: None,
            default_max_distance: None,
            distance_metric: EmbeddingDistanceMetric::default(),
            base_url: None,
            quantization: EmbeddingQuantization::default(),
        }
    }

    /// Check if this config uses the unified AI provider system.
    ///
    /// Returns true if `ai_provider_ref` is set, meaning the embedding
    /// provider should be resolved from TenantAIConfig.
    pub fn uses_unified_provider(&self) -> bool {
        self.ai_provider_ref.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config_defaults() {
        let config = TenantEmbeddingConfig::new("test-tenant".to_string());

        assert_eq!(config.tenant_id, "test-tenant");
        assert!(!config.enabled);
        assert!(config.ai_provider_ref.is_none());
        assert!(config.ai_model_ref.is_none());
        assert_eq!(config.provider, EmbeddingProvider::OpenAI);
        assert_eq!(config.model, "text-embedding-3-small");
        assert_eq!(config.dimensions, 1536);
        assert!(config.api_key_encrypted.is_none());
        assert!(config.include_name);
        assert!(config.include_path);
        assert!(config.max_embeddings_per_repo.is_none());
        assert!(config.chunking.is_none());
        assert!(!config.uses_unified_provider());
    }

    #[test]
    fn test_uses_unified_provider() {
        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());
        assert!(!config.uses_unified_provider());

        config.ai_provider_ref = Some("openai".to_string());
        config.ai_model_ref = Some("text-embedding-3-small".to_string());
        assert!(config.uses_unified_provider());
    }

    #[test]
    fn test_serialization() {
        let config = TenantEmbeddingConfig::new("test-tenant".to_string());
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: TenantEmbeddingConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.tenant_id, deserialized.tenant_id);
        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.provider, deserialized.provider);
    }

    #[test]
    fn test_messagepack_serialization() {
        let config = TenantEmbeddingConfig::new("test-tenant".to_string());

        // Serialize to MessagePack with field names (more robust to field order changes)
        let bytes = rmp_serde::to_vec_named(&config).unwrap();

        // Debug: print the bytes
        eprintln!("Serialized bytes length: {}", bytes.len());
        eprintln!(
            "First 20 bytes: {:?}",
            &bytes[..std::cmp::min(20, bytes.len())]
        );

        // Deserialize from MessagePack
        let deserialized: TenantEmbeddingConfig = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(config.tenant_id, deserialized.tenant_id);
        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.provider, deserialized.provider);
        assert_eq!(config.model, deserialized.model);
        assert_eq!(config.dimensions, deserialized.dimensions);
    }

    #[test]
    fn test_messagepack_with_all_fields() {
        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());
        config.enabled = true;
        config.api_key_encrypted = Some(vec![1, 2, 3, 4, 5]);
        config.max_embeddings_per_repo = Some(10000);

        // Serialize to MessagePack with field names (more robust to field order changes)
        let bytes = rmp_serde::to_vec_named(&config).unwrap();

        // Deserialize from MessagePack
        let deserialized: TenantEmbeddingConfig = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(config.tenant_id, deserialized.tenant_id);
        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.api_key_encrypted, deserialized.api_key_encrypted);
        assert_eq!(
            config.max_embeddings_per_repo,
            deserialized.max_embeddings_per_repo
        );
    }

    #[test]
    fn test_messagepack_with_chunking() {
        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());
        config.enabled = true;
        config.ai_provider_ref = Some("openai".to_string());
        config.ai_model_ref = Some("text-embedding-3-small".to_string());
        config.chunking = Some(raisin_ai::config::ChunkingConfig::default());

        // Serialize to MessagePack with field names
        let bytes = rmp_serde::to_vec_named(&config).unwrap();

        // Deserialize from MessagePack
        let deserialized: TenantEmbeddingConfig = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(config.tenant_id, deserialized.tenant_id);
        assert_eq!(config.ai_provider_ref, deserialized.ai_provider_ref);
        assert_eq!(config.ai_model_ref, deserialized.ai_model_ref);
        assert!(deserialized.chunking.is_some());
        assert_eq!(
            config.chunking.as_ref().unwrap().chunk_size,
            deserialized.chunking.as_ref().unwrap().chunk_size
        );
    }
}
