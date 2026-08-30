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
}

impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            include_name: true,
            include_path: true,
            max_embeddings_per_repo: None,
            dimensions: 1536,
            chunking: None,
        }
    }
}
