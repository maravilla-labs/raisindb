//! Text chunking for embedding pipelines.
//!
//! This module provides utilities for splitting large documents into smaller chunks
//! suitable for embedding generation. It supports multiple splitting strategies:
//!
//! - **Recursive**: Hierarchical splitting (paragraphs → sentences → words)
//! - **FixedSize**: Simple fixed-size chunks without semantic awareness
//! - **Markdown**: Markdown-aware splitting that respects headers and code blocks (future)
//! - **Code**: Code-aware splitting that respects function boundaries (future)
//!
//! # Example
//!
//! ```rust,ignore
//! use raisin_ai::chunking::TextChunker;
//! use raisin_ai::config::{ChunkingConfig, SplitterType, OverlapConfig};
//!
//! let config = ChunkingConfig {
//!     chunk_size: 256,
//!     overlap: OverlapConfig::Tokens(64),
//!     splitter: SplitterType::Recursive,
//!     tokenizer_id: None,
//! };
//!
//! let text = "Large document to be chunked...";
//! let chunks = TextChunker::chunk_text(text, &config).unwrap();
//!
//! for chunk in chunks {
//!     println!("Chunk {}: {} tokens", chunk.index, chunk.token_count);
//! }
//! ```

use crate::config::ChunkingConfig;
use text_splitter::{ChunkConfig, ChunkSizer, TextSplitter};

/// Represents a single text chunk with metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    /// The text content of the chunk.
    pub content: String,

    /// Zero-based index of this chunk in the sequence.
    pub index: usize,

    /// Estimated token count for this chunk.
    pub token_count: usize,

    /// Start byte offset in the original text.
    pub start_offset: usize,

    /// End byte offset in the original text (exclusive).
    pub end_offset: usize,
}

/// Text chunker that splits text according to configuration.
pub struct TextChunker;

impl TextChunker {
    /// Chunk text according to the provided configuration.
    ///
    /// # Arguments
    ///
    /// * `text` - The text to be chunked
    /// * `config` - Chunking configuration specifying chunk size, overlap, and splitter type
    ///
    /// # Returns
    ///
    /// A vector of `TextChunk` instances, each with metadata about position and token count.
    ///
    /// # Errors
    ///
    /// Returns `ChunkingError` if:
    /// - The tokenizer cannot be initialized
    /// - The chunking operation fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = ChunkingConfig::default();
    /// let chunks = TextChunker::chunk_text("Some long text...", &config)?;
    /// assert!(!chunks.is_empty());
    /// ```
    pub fn chunk_text(
        text: &str,
        config: &ChunkingConfig,
    ) -> Result<Vec<TextChunk>, ChunkingError> {
        // Handle empty input
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let overlap = config.overlap_tokens();

        // Create chunks based on splitter type and tokenizer availability.
        //
        // NOTE: `config.splitter_type` is deliberately NOT consulted here. Both
        // paths below use `text_splitter`, which is recursive regardless, so the
        // setting currently changes the spec hash and nothing else. Making it
        // take effect would move every split and therefore every `spec_hash`,
        // re-embedding every document in the fleet — that needs its own
        // decision, not a silent fix wedged into an offset change.
        let chunk_spans = if let Some(ref tokenizer_id) = config.tokenizer_id {
            Self::chunk_with_tiktoken(text, config.chunk_size, overlap, tokenizer_id)?
        } else {
            Self::chunk_char_based(text, config.chunk_size, overlap)?
        };

        // Offsets come FROM THE SPLITTER, which already knows them.
        //
        // This used to re-find each chunk in the source with
        // `text[current_offset..].find(&content)`, advancing `current_offset` by
        // one byte per chunk. Three things were wrong with that, and the first
        // is a crash:
        //
        //   1. `text[current_offset..]` PANICS when `current_offset` is not a
        //      char boundary — and it was `previous_start + 1`, which is a
        //      non-boundary whenever a chunk starts with a multi-byte character.
        //      Any accented word at a chunk boundary took the embedding job down.
        //   2. A one-byte-per-chunk search window means repeated text — a running
        //      header, boilerplate, a repeated heading — resolves to an EARLIER
        //      occurrence, so the offsets point at the wrong passage.
        //   3. A missed `find` (the splitter trims chunk edges in some
        //      configurations) silently fell back to an offset that does not
        //      correspond to the content at all.
        //
        // `text_splitter::chunks()` is literally
        // `chunk_indices(text).map(|(_, t)| t)`, so the byte offsets were being
        // computed by the splitter, thrown away, and then reconstructed badly.
        // Taking them directly is exact, total, and less code.
        let chunks: Vec<TextChunk> = chunk_spans
            .into_iter()
            .enumerate()
            .map(|(index, (start_offset, content))| {
                let end_offset = start_offset + content.len();

                let token_count = if config.tokenizer_id.is_some() {
                    // Estimate based on actual tokenizer if we have one
                    Self::estimate_token_count(&content)
                } else {
                    // Rough estimate: ~4 chars per token
                    content.len() / 4
                };

                TextChunk {
                    content,
                    index,
                    token_count,
                    start_offset,
                    end_offset,
                }
            })
            .collect();

        Ok(chunks)
    }

    /// Chunk text using tiktoken-based tokenizer for accurate token counting.
    ///
    /// Returns `(byte offset into `text`, chunk)` pairs — see the note in
    /// [`Self::chunk_text`] on why the offset comes from the splitter.
    #[cfg(feature = "tiktoken-rs")]
    fn chunk_with_tiktoken(
        text: &str,
        chunk_size: usize,
        overlap: usize,
        tokenizer_id: &str,
    ) -> Result<Vec<(usize, String)>, ChunkingError> {
        use tiktoken_rs::get_bpe_from_model;

        // Load the tokenizer
        let tokenizer = get_bpe_from_model(tokenizer_id).map_err(|e| {
            ChunkingError::TokenizerError(format!(
                "Failed to load tokenizer '{}': {}",
                tokenizer_id, e
            ))
        })?;

        // Create a custom sizer
        let sizer = TiktokenSizer { tokenizer };

        // Create chunk config with capacity
        let chunk_config = ChunkConfig::new(chunk_size)
            .with_overlap(overlap)
            .map_err(|e| {
                ChunkingError::ConfigError(format!("Invalid overlap configuration: {}", e))
            })?;

        // Create splitter
        let splitter = TextSplitter::new(chunk_config.with_sizer(sizer));

        // Split and collect chunks WITH their byte offsets.
        let chunks: Vec<(usize, String)> = splitter
            .chunk_indices(text)
            .map(|(start, s)| (start, s.to_string()))
            .collect();

        Ok(chunks)
    }

    /// Fallback implementation when tiktoken feature is not enabled.
    #[cfg(not(feature = "tiktoken-rs"))]
    fn chunk_with_tiktoken(
        text: &str,
        chunk_size: usize,
        overlap: usize,
        _tokenizer_id: &str,
    ) -> Result<Vec<(usize, String)>, ChunkingError> {
        tracing::warn!(
            "Tiktoken requested but feature not enabled, falling back to character-based chunking"
        );
        Self::chunk_char_based(text, chunk_size, overlap)
    }

    /// Chunk text using character-based counting (fallback when no tokenizer specified).
    ///
    /// Returns `(byte offset into `text`, chunk)` pairs — see the note in
    /// [`Self::chunk_text`] on why the offset comes from the splitter.
    fn chunk_char_based(
        text: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<Vec<(usize, String)>, ChunkingError> {
        use text_splitter::Characters;

        // Create a ChunkConfig with the specified size and overlap
        let chunk_config = ChunkConfig::new(chunk_size)
            .with_overlap(overlap)
            .map_err(|e| {
                ChunkingError::ConfigError(format!("Invalid overlap configuration: {}", e))
            })?;

        // Use character-based sizer (pass the sizer through with_sizer)
        let chunk_config_with_sizer = chunk_config.with_sizer(Characters);

        // Create splitter
        let splitter = TextSplitter::new(chunk_config_with_sizer);

        // Split and collect chunks WITH their byte offsets.
        let chunks: Vec<(usize, String)> = splitter
            .chunk_indices(text)
            .map(|(start, s)| (start, s.to_string()))
            .collect();

        Ok(chunks)
    }

    /// Estimate token count for a text chunk.
    ///
    /// This is a rough estimation used when we don't want to tokenize every chunk.
    /// Rule of thumb: ~4 characters per token for English text.
    fn estimate_token_count(text: &str) -> usize {
        // More sophisticated estimation could be added here
        // For now, use a simple heuristic
        let char_count = text.chars().count();
        (char_count / 4).max(1)
    }
}

// =============================================================================
// Tiktoken Sizer Implementation
// =============================================================================

/// A chunk sizer that uses tiktoken for accurate token counting.
#[cfg(feature = "tiktoken-rs")]
struct TiktokenSizer {
    tokenizer: tiktoken_rs::CoreBPE,
}

#[cfg(feature = "tiktoken-rs")]
impl ChunkSizer for TiktokenSizer {
    /// Returns the size of the given chunk in tokens.
    fn size(&self, chunk: &str) -> usize {
        self.tokenizer.encode_ordinary(chunk).len()
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during text chunking.
#[derive(Debug, thiserror::Error)]
pub enum ChunkingError {
    /// Failed to initialize or use tokenizer.
    #[error("Tokenizer error: {0}")]
    TokenizerError(String),

    /// Invalid chunking configuration.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Chunking operation failed.
    #[error("Chunking failed: {0}")]
    ChunkingFailed(String),
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OverlapConfig;

    #[test]
    fn test_empty_text() {
        let config = ChunkingConfig::default();
        let result = TextChunker::chunk_text("", &config);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    #[cfg(feature = "tiktoken-rs")]
    fn test_recursive_chunking_with_tokenizer() {
        let config = ChunkingConfig {
            chunk_size: 50,
            overlap: OverlapConfig::Tokens(10),
            splitter: crate::config::SplitterType::Recursive,
            tokenizer_id: Some("gpt-3.5-turbo".to_string()),
        };

        let text = "This is a test document. ".repeat(20);
        let result = TextChunker::chunk_text(&text, &config);

        assert!(result.is_ok());
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());

        // Verify metadata
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.index, i);
            assert!(chunk.token_count > 0);
            assert!(chunk.start_offset < chunk.end_offset);
            assert!(!chunk.content.is_empty());
        }
    }

    #[test]
    fn test_char_based_chunking() {
        let config = ChunkingConfig {
            chunk_size: 30,
            overlap: OverlapConfig::Tokens(5),
            splitter: crate::config::SplitterType::FixedSize,
            tokenizer_id: None, // Use char-based fallback
        };

        let text = "Short text for testing fixed-size chunking behavior.";
        let result = TextChunker::chunk_text(&text, &config);

        assert!(result.is_ok());
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_percentage_overlap() {
        let config = ChunkingConfig {
            chunk_size: 100,
            overlap: OverlapConfig::Percentage(0.2),
            splitter: crate::config::SplitterType::Recursive,
            tokenizer_id: None,
        };

        let text = "Lorem ipsum dolor sit amet. ".repeat(30);
        let result = TextChunker::chunk_text(&text, &config);

        assert!(result.is_ok());
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());

        // Verify overlap calculation
        assert_eq!(config.overlap_tokens(), 20); // 20% of 100
    }

    #[test]
    fn test_small_text_no_chunking_needed() {
        let config = ChunkingConfig {
            chunk_size: 1000,
            overlap: OverlapConfig::Tokens(0),
            splitter: crate::config::SplitterType::Recursive,
            tokenizer_id: None,
        };

        let text = "Small text.";
        let result = TextChunker::chunk_text(&text, &config);

        assert!(result.is_ok());
        let chunks = result.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
        assert_eq!(chunks[0].index, 0);
    }

    #[test]
    fn test_chunk_offsets_sequential() {
        let config = ChunkingConfig {
            chunk_size: 20,
            overlap: OverlapConfig::Tokens(5),
            splitter: crate::config::SplitterType::Recursive,
            tokenizer_id: None,
        };

        let text = "First sentence here. Second sentence here. Third sentence here.";
        let result = TextChunker::chunk_text(&text, &config);

        assert!(result.is_ok());
        let chunks = result.unwrap();

        // Verify offsets are within bounds
        for chunk in &chunks {
            assert!(chunk.start_offset < text.len());
            assert!(chunk.end_offset <= text.len());
            assert!(chunk.start_offset < chunk.end_offset);

            // Verify content matches offset
            let extracted = &text[chunk.start_offset..chunk.end_offset];
            assert_eq!(extracted, chunk.content);
        }
    }

    #[test]
    fn test_estimate_token_count() {
        let text =
            "This is a simple test sentence with about twenty words in it for testing purposes.";
        let estimate = TextChunker::estimate_token_count(text);

        // Should be roughly len/4
        let expected = text.len() / 4;
        assert!(estimate >= expected / 2 && estimate <= expected * 2);
    }
}

/// Chunk offsets must be exact, and must not crash.
///
/// The offsets used to be reconstructed by re-finding each chunk in the source
/// with a one-byte-per-chunk search window. Both cases below defeated that:
/// a multi-byte character at a chunk boundary made the slice panic, and
/// repeated text resolved to an earlier occurrence.
#[cfg(test)]
mod offset_tests {
    use super::*;
    use crate::config::{ChunkingConfig, OverlapConfig, SplitterType};

    fn config(chunk_size: usize) -> ChunkingConfig {
        ChunkingConfig {
            chunk_size,
            overlap: OverlapConfig::Tokens(0),
            splitter: SplitterType::Recursive,
            tokenizer_id: None,
        }
    }

    /// Every chunk's offsets must slice back to exactly that chunk. This is the
    /// property the whole chunk-retrieval feature rests on.
    fn assert_offsets_round_trip(text: &str, chunks: &[TextChunk]) {
        for chunk in chunks {
            assert!(
                chunk.end_offset <= text.len(),
                "chunk {} ends past the text ({} > {})",
                chunk.index,
                chunk.end_offset,
                text.len()
            );
            assert!(
                text.is_char_boundary(chunk.start_offset)
                    && text.is_char_boundary(chunk.end_offset),
                "chunk {} offsets {}..{} are not char boundaries",
                chunk.index,
                chunk.start_offset,
                chunk.end_offset
            );
            assert_eq!(
                &text[chunk.start_offset..chunk.end_offset],
                chunk.content,
                "chunk {} does not slice back to itself",
                chunk.index
            );
        }
    }

    /// Multi-byte characters at chunk boundaries. The old implementation
    /// PANICKED here: it sliced `text[previous_start + 1..]`, which is not a
    /// char boundary when a chunk starts with a multi-byte character.
    #[test]
    fn multibyte_text_does_not_panic_and_offsets_are_exact() {
        let text = "Schönheit größer Übermaß. Käse mit Grüßen und Straße.                     Ärger über Öl. Führung möglich. Prüfung dafür.";
        let chunks = TextChunker::chunk_text(text, &config(20)).expect("chunking must succeed");
        assert!(!chunks.is_empty());
        assert_offsets_round_trip(text, &chunks);
    }

    /// Emoji are 4-byte, which is the harshest boundary case.
    #[test]
    fn astral_plane_characters_are_handled() {
        let text = "🚀 launch sequence 🚀 second stage 🚀 orbit reached 🚀 done";
        let chunks = TextChunker::chunk_text(text, &config(12)).expect("chunking must succeed");
        assert_offsets_round_trip(text, &chunks);
    }

    /// Repeated text — a running header, boilerplate, a repeated heading. The
    /// old search window advanced one byte per chunk, so a later chunk whose
    /// text also appears earlier resolved to the EARLIER occurrence and the
    /// offsets pointed at the wrong passage.
    #[test]
    fn repeated_passages_resolve_to_their_own_occurrence() {
        let header = "CONFIDENTIAL DRAFT";
        let text = format!(
            "{header}\n\nAlpha section body text.\n\n             {header}\n\nBeta section body text.\n\n             {header}\n\nGamma section body text."
        );

        let chunks = TextChunker::chunk_text(&text, &config(30)).expect("chunking must succeed");
        assert_offsets_round_trip(&text, &chunks);

        // Offsets must be non-decreasing: a chunk may not start before the one
        // before it, which is exactly what re-finding repeated text produced.
        for pair in chunks.windows(2) {
            assert!(
                pair[1].start_offset >= pair[0].start_offset,
                "chunk {} starts at {}, before chunk {} at {} — a repeated \
                 passage resolved to an earlier occurrence",
                pair[1].index,
                pair[1].start_offset,
                pair[0].index,
                pair[0].start_offset
            );
        }
    }

    /// With overlap configured, consecutive chunks SHARE source bytes. Anything
    /// stitching neighbours must merge ranges rather than concatenate text, or
    /// it duplicates the overlap.
    #[test]
    fn overlapping_chunks_share_source_ranges() {
        let cfg = ChunkingConfig {
            chunk_size: 40,
            overlap: OverlapConfig::Tokens(10),
            splitter: SplitterType::Recursive,
            tokenizer_id: None,
        };
        let text = "one two three four five six seven eight nine ten eleven twelve \
                    thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
        let chunks = TextChunker::chunk_text(text, &cfg).expect("chunking must succeed");
        assert_offsets_round_trip(text, &chunks);
        assert!(
            chunks.len() > 1,
            "expected several chunks, got {}",
            chunks.len()
        );
    }
}
