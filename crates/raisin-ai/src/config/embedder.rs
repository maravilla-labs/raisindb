//! Embedder identity and embedding settings.

use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};

use super::ChunkingConfig;

/// Unique identifier for an embedding configuration.
///
/// Used to separate vector indexes when the embedding model/provider changes.
/// This prevents collision between embeddings from different models in storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EmbedderId {
    /// Provider name (e.g., "openai", "ollama").
    pub provider: String,

    /// Model identifier (e.g., "text-embedding-3-small").
    pub model: String,

    /// Vector dimensionality (e.g., 1536).
    pub dimensions: usize,

    /// Tokenizer identifier for chunking consistency.
    /// Important: different tokenizers produce different chunk boundaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_id: Option<String>,
}

impl EmbedderId {
    /// Create a new embedder identity.
    pub fn new(provider: impl Into<String>, model: impl Into<String>, dimensions: usize) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            dimensions,
            tokenizer_id: None,
        }
    }

    /// Set the tokenizer ID.
    pub fn with_tokenizer(mut self, tokenizer_id: impl Into<String>) -> Self {
        self.tokenizer_id = Some(tokenizer_id.into());
        self
    }

    /// Generate a stable, short hash for use in storage keys.
    ///
    /// Returns an 11-character base64url-encoded hash (8 bytes).
    /// This is collision-resistant for practical purposes while keeping keys compact.
    pub fn to_key_hash(&self) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;

        let input = format!(
            "{}:{}:{}:{}",
            self.provider,
            self.model,
            self.dimensions,
            self.tokenizer_id.as_deref().unwrap_or("")
        );
        let hash = digest(&SHA256, input.as_bytes());
        // Take first 8 bytes for compact key (64 bits = plenty for this use case)
        URL_SAFE_NO_PAD.encode(&hash.as_ref()[..8])
    }
}

/// Type of embedding content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingKind {
    /// Text content embedding (from node properties, PDF text, etc.).
    Text,

    /// Image embedding (from CLIP or similar vision model).
    Image,
}

impl EmbeddingKind {
    /// Single-character representation for storage keys.
    pub fn to_key_char(&self) -> char {
        match self {
            EmbeddingKind::Text => 'T',
            EmbeddingKind::Image => 'I',
        }
    }

    /// Parse from single-character key representation.
    pub fn from_key_char(c: char) -> Option<Self> {
        match c {
            'T' => Some(EmbeddingKind::Text),
            'I' => Some(EmbeddingKind::Image),
            _ => None,
        }
    }
}

/// Which vector space a set of embeddings lives in: an embedder plus a kind.
///
/// This exists so that the `cf::EMBEDDINGS` key and the HNSW index file name are
/// derived from ONE thing. The storage key has always carried both segments:
///
/// ```text
/// {tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash}\0{kind}\0{source}\0{chunk}\0{rev}
///                                          ^^^^^^^^^^^^^^^   ^^^^
///                                          segments 5 and 6
/// ```
///
/// The index did not, so there was one index per branch and turning on a second
/// embedder made that index unloadable for BOTH — text search goes down when
/// image search comes up. The genuinely silent case is two embedders of the
/// SAME width: nothing compares anything but `dimensions`, so both models land
/// in one graph, every distance is finite and every ranking is confident
/// nonsense. No width check can catch that; only partitioning can.
///
/// # One rendering
///
/// [`Self::to_index_token`] is the ONLY place a partition is turned into a
/// string. `raisin-hnsw` cannot see these types (its only raisin dependencies
/// are `raisin-error` and `raisin-hlc`, deliberately — `raisin-ai` pulls candle
/// and tesseract), so it takes the rendered token as an opaque
/// `PartitionId`. `raisin-embeddings` has a test asserting that this token's
/// bytes are exactly segments 5 and 6 of the key its own writer builds. That
/// test is what stops the two from drifting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmbeddingPartition {
    /// Which model produced the vectors.
    pub embedder: EmbedderId,
    /// Text or image.
    pub kind: EmbeddingKind,
}

impl EmbeddingPartition {
    /// A partition for one embedder and kind.
    pub fn new(embedder: EmbedderId, kind: EmbeddingKind) -> Self {
        Self { embedder, kind }
    }

    /// The text partition of an embedder — by far the common case.
    pub fn text(embedder: EmbedderId) -> Self {
        Self::new(embedder, EmbeddingKind::Text)
    }

    /// The image partition of an embedder.
    pub fn image(embedder: EmbedderId) -> Self {
        Self::new(embedder, EmbeddingKind::Image)
    }

    /// `{embedder_hash}{kind_char}` — the HNSW index file stem.
    ///
    /// Byte-identical to segments 5 and 6 of the `cf::EMBEDDINGS` key
    /// concatenated. Safe as a file stem: the hash alphabet is
    /// `URL_SAFE_NO_PAD` base64 (`A-Za-z0-9-_`, no `.` and no `/`) and the kind
    /// is one ASCII letter.
    pub fn to_index_token(&self) -> String {
        let mut token = self.embedder.to_key_hash();
        token.push(self.kind.to_key_char());
        token
    }
}

/// Settings specific to embedding generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSettings {
    /// Whether embeddings are enabled for this tenant
    pub enabled: bool,
    /// Slug of the `TenantAIConfig` provider entry that generates embeddings.
    ///
    /// The console has sent this since the slug-based picker shipped, and this
    /// struct had nowhere to put it — so serde dropped it on every save and the
    /// form redrew as "Select a provider..." while `enabled` and `dimensions`,
    /// which ARE fields here, persisted. A partially-saved document is worse
    /// than a rejected one: it looks configured and cannot resolve an embedder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_provider_ref: Option<String>,
    /// The model id within that provider, e.g. `bge_multilingual_gemma2`.
    /// Dropped for the same reason and with the same consequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_model_ref: Option<String>,
    /// Whether to include node name in embedding content
    pub include_name: bool,
    /// Whether to include node path in embedding content
    pub include_path: bool,
    /// Maximum number of embeddings allowed per repository
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_embeddings_per_repo: Option<usize>,
    /// Vector dimensionality (e.g., 1536 for text-embedding-3-small)
    pub dimensions: usize,
    /// Chunking configuration for splitting large text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunking: Option<ChunkingConfig>,
    /// Default max distance for vector search; results beyond it are dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_max_distance: Option<f32>,
    /// Distance metric the HNSW index is BUILT and queried under, carried
    /// verbatim as the string the console sends.
    ///
    /// Deliberately NOT typed as an enum here. The one definition of these
    /// variants lives in `raisin_embeddings::config::EmbeddingDistanceMetric`,
    /// and `raisin-embeddings` depends on `raisin-ai` -- not the other way
    /// round -- so typing it here would mean a second spelling of the same
    /// enum in the crate that cannot see the first. Two spellings of one enum
    /// is the drift this codebase keeps paying for. The value is parsed against
    /// the real enum where it lands, so an unknown string fails there rather
    /// than becoming a silently different metric.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_metric: Option<String>,
    /// Scalar precision the index stores vectors at (`F32` / `F16` / `Int8`),
    /// carried as a string for the same reason as `distance_metric`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
}

impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            ai_provider_ref: None,
            ai_model_ref: None,
            include_name: true,
            include_path: true,
            max_embeddings_per_repo: None,
            dimensions: 1536,
            chunking: None,
            default_max_distance: None,
            distance_metric: None,
            quantization: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The console has always sent these five keys. Until they were fields here,
    /// serde dropped them on the way in and kept `enabled` and `dimensions` --
    /// so a saved config redrew with no provider selected while still reporting
    /// itself enabled, and nothing downstream could name an embedder.
    #[test]
    fn a_console_payload_keeps_every_field_it_sends() {
        let payload = serde_json::json!({
            "enabled": true,
            "ai_provider_ref": "marvel",
            "ai_model_ref": "bge_multilingual_gemma2",
            "include_name": true,
            "include_path": true,
            "dimensions": 3584,
            "default_max_distance": 0.6,
            "distance_metric": "Cosine",
            "quantization": "F32"
        });

        let settings: EmbeddingSettings = serde_json::from_value(payload).expect("deserializes");

        assert_eq!(settings.ai_provider_ref.as_deref(), Some("marvel"));
        assert_eq!(
            settings.ai_model_ref.as_deref(),
            Some("bge_multilingual_gemma2")
        );
        assert_eq!(settings.dimensions, 3584);
        assert_eq!(settings.distance_metric.as_deref(), Some("Cosine"));
        assert_eq!(settings.quantization.as_deref(), Some("F32"));

        // And they survive the write back out, or the next GET would show the
        // form empty again for a different reason.
        let round_tripped: EmbeddingSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(round_tripped.ai_provider_ref.as_deref(), Some("marvel"));
    }

    /// A config stored before these fields existed must still load.
    #[test]
    fn a_pre_slug_document_still_deserializes() {
        let settings: EmbeddingSettings = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "include_name": true,
            "include_path": true,
            "dimensions": 1536
        }))
        .expect("deserializes");

        assert!(settings.ai_provider_ref.is_none());
        assert!(settings.quantization.is_none());
    }
}
