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

impl ChunkingConfig {
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
