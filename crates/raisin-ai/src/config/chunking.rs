//! Chunking configuration for text splitting in embedding pipelines.

use serde::{Deserialize, Serialize};

/// Configuration for text chunking in embedding pipelines.
///
/// Controls how large documents are split into smaller chunks for embedding.
/// Based on langchain-style chunking patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// Target chunk size in tokens.
    /// Default: 256 (safe for 512-token context limit models)
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,

    /// Overlap configuration between adjacent chunks.
    /// Ensures context is not lost at chunk boundaries.
    #[serde(default)]
    pub overlap: OverlapConfig,

    /// Type of text splitter to use.
    #[serde(default)]
    pub splitter: SplitterType,

    /// Optional tokenizer identifier for accurate token counting.
    /// If None, uses a default tokenizer based on the embedding model.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tokenizer_id: Option<String>,
}

fn default_chunk_size() -> usize {
    256
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            chunk_size: 256,
            overlap: OverlapConfig::Tokens(64),
            splitter: SplitterType::Recursive,
            tokenizer_id: None,
        }
    }
}

/// The tokenizer used to COUNT chunk sizes when none was configured.
///
/// It is a counting proxy, deliberately not "the embedding model's own
/// tokenizer": a tenant on Ollama or Voyage has a model name tiktoken has never
/// heard of, and asking for it by name fails to load. Every modern text
/// embedder tokenizes within a small factor of cl100k, and a chunk size is a
/// heuristic, so a stable proxy that always loads beats a per-model lookup that
/// sometimes doesn't. Naming a model here rather than an encoding is what
/// `tiktoken_rs::get_bpe_from_model` accepts.
pub const COUNTING_TOKENIZER: &str = "text-embedding-3-small";

impl ChunkingConfig {
    /// The chunking a DOCUMENT BODY gets when nothing else is configured.
    ///
    /// This is the fallback for the `doc` spec — the text extracted out of a
    /// PDF or handed back by a converter plugin. It exists because the
    /// alternative default is worse than it looks: with `chunking: None` a
    /// forty-page contract becomes ONE vector, which is a near-neighbour of
    /// nothing in particular, and retrieval quietly returns the wrong document
    /// with no error anywhere.
    ///
    /// Sizes here are TOKENS, because `tokenizer_id` is set. Without one the
    /// same numbers would be counted in CHARACTERS — the default 256 meaning
    /// 256 characters, roughly a sentence and a half, which is why an
    /// unconfigured install that did switch chunking on still retrieved badly.
    ///
    /// 512/64 fits the context limit of every embedder we resolve to while
    /// leaving a clause intact across a page break.
    pub fn for_documents() -> Self {
        Self {
            chunk_size: 512,
            overlap: OverlapConfig::Tokens(64),
            splitter: SplitterType::Recursive,
            tokenizer_id: Some(COUNTING_TOKENIZER.to_string()),
        }
    }

    /// Calculate effective overlap in tokens based on chunk_size.
    pub fn overlap_tokens(&self) -> usize {
        match self.overlap {
            OverlapConfig::Tokens(n) => n,
            OverlapConfig::Percentage(pct) => {
                ((self.chunk_size as f32) * pct.clamp(0.0, 0.5)) as usize
            }
        }
    }
}

/// Overlap configuration between adjacent chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum OverlapConfig {
    /// Fixed number of tokens overlap (e.g., 64 tokens).
    Tokens(usize),

    /// Percentage of chunk_size (e.g., 0.2 = 20% overlap).
    /// Clamped to 0.0-0.5 range.
    Percentage(f32),
}

impl Default for OverlapConfig {
    fn default() -> Self {
        OverlapConfig::Tokens(64)
    }
}

/// Type of text splitter algorithm.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitterType {
    /// Recursive splitting: paragraphs -> sentences -> words.
    /// Best for most documents, preserves semantic structure.
    #[default]
    Recursive,

    /// Simple fixed-size chunks with no semantic awareness.
    /// Fastest, but may cut mid-sentence.
    FixedSize,

    /// Markdown-aware splitting (respects headers, code blocks).
    Markdown,

    /// Code-aware splitting (respects function boundaries).
    Code,
}
