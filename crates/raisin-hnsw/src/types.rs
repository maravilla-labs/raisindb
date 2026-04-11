// SPDX-License-Identifier: BSL-1.1

//! Type definitions for HNSW engine.

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Distance metric for HNSW vector similarity search.
///
/// Determines how distance between vectors is computed in the HNSW index.
/// The metric is fixed at index creation time; changing it requires a full rebuild.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMetric {
    /// Cosine distance: `1 - cosine_similarity(a, b)`.
    /// Vectors should be normalized to unit length for correct results.
    /// Range: 0.0 (identical) to 2.0 (opposite).
    #[default]
    Cosine,

    /// Euclidean (L2) distance: `sqrt(sum((a_i - b_i)^2))`.
    /// Do NOT normalize vectors when using L2.
    L2,

    /// Inner product distance: `1 - dot(a, b)`.
    /// Vectors should be normalized for consistent results.
    InnerProduct,

    /// Hamming distance: count of differing dimensions.
    /// Typically used with binary vectors.
    Hamming,
}

impl fmt::Display for DistanceMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DistanceMetric::Cosine => write!(f, "Cosine"),
            DistanceMetric::L2 => write!(f, "L2"),
            DistanceMetric::InnerProduct => write!(f, "InnerProduct"),
            DistanceMetric::Hamming => write!(f, "Hamming"),
        }
    }
}

impl DistanceMetric {
    /// Whether vectors should be normalized before indexing/searching with this metric.
    pub fn requires_normalization(&self) -> bool {
        matches!(self, DistanceMetric::Cosine | DistanceMetric::InnerProduct)
    }
}

/// Vector quantization type for HNSW indexes.
///
/// Controls the precision of stored vectors, trading accuracy for memory savings.
/// Lower precision types use less memory per vector but may reduce search quality.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantizationType {
    /// 32-bit floating point (default). Full precision, no compression.
    #[default]
    F32,

    /// 16-bit floating point. ~50% memory reduction with minimal accuracy loss.
    F16,

    /// 8-bit signed integer. ~75% memory reduction, suitable for large indexes
    /// where some accuracy loss is acceptable.
    Int8,
}

impl fmt::Display for QuantizationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantizationType::F32 => write!(f, "F32"),
            QuantizationType::F16 => write!(f, "F16"),
            QuantizationType::Int8 => write!(f, "Int8"),
        }
    }
}

/// HNSW index tuning parameters.
///
/// Controls the accuracy/speed/memory tradeoff of the HNSW graph.
/// Zero values mean "use library defaults" (usearch defaults are generally good).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HnswParams {
    /// Graph connectivity (M parameter). Higher = more accurate, more memory.
    /// Default: 0 (usearch default, typically 16).
    /// Recommended range: 4-64.
    #[serde(default)]
    pub connectivity: usize,

    /// Construction expansion factor (ef_construction). Higher = better index quality, slower build.
    /// Default: 0 (usearch default, typically 128).
    /// Recommended range: 100-500.
    #[serde(default)]
    pub expansion_add: usize,

    /// Search expansion factor (ef_search). Higher = more accurate search, slower.
    /// Default: 0 (usearch default, typically 64).
    /// Recommended range: 10-500.
    #[serde(default)]
    pub expansion_search: usize,

    /// Vector quantization type. Controls precision vs memory tradeoff.
    /// Default: F32 (full precision).
    #[serde(default)]
    pub quantization: QuantizationType,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self {
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            quantization: QuantizationType::default(),
        }
    }
}

/// A point in vector space with associated node metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorPoint {
    /// Node identifier
    pub node_id: String,

    /// Workspace identifier
    pub workspace_id: String,

    /// Revision (full HLC with timestamp and counter)
    pub revision: HLC,

    /// Embedding vector
    pub vector: Vec<f32>,
}

impl VectorPoint {
    /// Create a new vector point
    pub fn new(node_id: String, workspace_id: String, revision: HLC, vector: Vec<f32>) -> Self {
        Self {
            node_id,
            workspace_id,
            revision,
            vector,
        }
    }
}

/// Search result with distance metric.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Node identifier
    pub node_id: String,

    /// Workspace identifier
    pub workspace_id: String,

    /// Revision (full HLC with timestamp and counter)
    pub revision: HLC,

    /// Distance from query vector (lower is better)
    /// For cosine similarity: 0.0 = identical, 2.0 = opposite
    pub distance: f32,
}

impl SearchResult {
    /// Create a new search result
    pub fn new(node_id: String, workspace_id: String, revision: HLC, distance: f32) -> Self {
        Self {
            node_id,
            workspace_id,
            revision,
            distance,
        }
    }

    /// Convert distance to similarity score (0.0 = dissimilar, 1.0 = identical)
    pub fn similarity(&self) -> f32 {
        1.0 - (self.distance / 2.0).min(1.0)
    }
}

/// Search result mode for multi-chunk results.
///
/// When documents are chunked into multiple embeddings, this determines
/// how results should be returned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchMode {
    /// Return all matching chunks, ranked by similarity.
    ///
    /// Each chunk from each document is returned as a separate result,
    /// allowing you to see all relevant sections across all documents.
    Chunks,

    /// Return best chunk per source document, deduplicated.
    ///
    /// For each source document, only the most similar chunk is returned,
    /// preventing duplicate documents in results.
    #[default]
    Documents,
}

/// Scoring configuration for chunk-aware search.
///
/// This configuration allows you to adjust how search results are ranked
/// based on chunk position and other factors beyond just vector similarity.
#[derive(Debug, Clone)]
pub struct ScoringConfig {
    /// Weight decay for chunk position (0.0 = no decay, 1.0 = strong decay).
    ///
    /// Earlier chunks (lower index) get higher scores. For example, with
    /// `position_decay = 0.1`, chunk 0 gets 100% score, chunk 1 gets 90%,
    /// chunk 2 gets 80%, etc.
    ///
    /// This is useful because the first chunks of documents often contain
    /// the most important information (summary, introduction, etc.).
    pub position_decay: f32,

    /// Boost multiplier for the first chunk of each document.
    ///
    /// The first chunk often contains summary or introductory information
    /// that's especially relevant. A value of 1.2 means 20% boost for first chunks.
    pub first_chunk_boost: f32,

    /// Boost for exact phrase matches (reserved for future use).
    ///
    /// Currently not implemented. A value of 1.0 means no boost.
    pub exact_match_boost: f32,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            position_decay: 0.1,    // 10% decay per chunk position
            first_chunk_boost: 1.2, // 20% boost for first chunk
            exact_match_boost: 1.0, // No boost by default (not yet implemented)
        }
    }
}

/// Search request with mode and filtering options.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Query vector to search for
    pub query_vector: Vec<f32>,

    /// Maximum number of results to return
    pub k: usize,

    /// Search mode: return all chunks or deduplicate by document
    pub mode: SearchMode,

    /// Optional workspace filter (None = all workspaces)
    pub workspace_filter: Option<String>,

    /// Maximum distance threshold (0.0 = identical, 2.0 = opposite for cosine).
    ///
    /// Results with distance greater than or equal to this threshold will be filtered out.
    /// If None, uses the default MAX_DISTANCE constant (0.6).
    ///
    /// For cosine distance on normalized vectors:
    /// - 0.0-0.2: Very similar (cosine sim > 0.80)
    /// - 0.2-0.4: Similar (cosine sim 0.80-0.60)
    /// - 0.4-0.6: Weakly related (cosine sim 0.60-0.40)
    /// - 0.6+: Not related (cosine sim < 0.40)
    pub max_distance: Option<f32>,

    /// Scoring configuration for chunk-aware ranking.
    ///
    /// If None, results are ranked purely by vector similarity.
    /// If Some, applies position-based scoring adjustments.
    pub scoring: Option<ScoringConfig>,
}

impl SearchRequest {
    /// Create a new search request.
    ///
    /// # Arguments
    ///
    /// * `query_vector` - Query embedding vector
    /// * `k` - Maximum number of results to return
    pub fn new(query_vector: Vec<f32>, k: usize) -> Self {
        Self {
            query_vector,
            k,
            mode: SearchMode::default(),
            workspace_filter: None,
            max_distance: None,
            scoring: None,
        }
    }

    /// Set the search mode.
    pub fn with_mode(mut self, mode: SearchMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the workspace filter.
    pub fn with_workspace(mut self, workspace_id: String) -> Self {
        self.workspace_filter = Some(workspace_id);
        self
    }

    /// Set the maximum distance threshold.
    pub fn with_max_distance(mut self, max_distance: f32) -> Self {
        self.max_distance = Some(max_distance);
        self
    }

    /// Set the scoring configuration.
    pub fn with_scoring(mut self, scoring: ScoringConfig) -> Self {
        self.scoring = Some(scoring);
        self
    }
}

/// Search result for a single chunk.
///
/// Returned when using `SearchMode::Chunks`.
#[derive(Debug, Clone)]
pub struct ChunkSearchResult {
    /// Source document identifier (without chunk suffix)
    pub source_id: String,

    /// Workspace identifier
    pub workspace: String,

    /// Zero-based chunk index within the source document
    pub chunk_index: usize,

    /// Total number of chunks in the source document
    pub total_chunks: usize,

    /// Distance from query vector (lower is better)
    pub distance: f32,

    /// Optional text excerpt from this chunk
    pub excerpt: Option<String>,

    /// Full node_id (includes chunk information)
    pub node_id: String,

    /// Revision (full HLC with timestamp and counter)
    pub revision: HLC,

    /// Adjusted score after applying scoring config (if any).
    ///
    /// This field is populated when a ScoringConfig is provided in the search request.
    /// It represents the final ranking score after position decay and boosts are applied.
    pub adjusted_score: Option<f32>,
}

impl ChunkSearchResult {
    /// Create a new chunk search result from a basic SearchResult.
    ///
    /// # Arguments
    ///
    /// * `result` - The basic search result
    /// * `total_chunks` - Total chunks in the source document
    /// * `excerpt` - Optional text excerpt
    pub fn from_search_result(
        result: SearchResult,
        total_chunks: usize,
        excerpt: Option<String>,
    ) -> Self {
        let (source_id, chunk_index) = parse_chunk_id(&result.node_id);

        Self {
            source_id,
            workspace: result.workspace_id,
            chunk_index,
            total_chunks,
            distance: result.distance,
            excerpt,
            node_id: result.node_id,
            revision: result.revision,
            adjusted_score: None,
        }
    }

    /// Convert distance to similarity score (0.0 = dissimilar, 1.0 = identical)
    pub fn similarity(&self) -> f32 {
        1.0 - (self.distance / 2.0).min(1.0)
    }
}

/// Search result for a source document (best chunk).
///
/// Returned when using `SearchMode::Documents`.
#[derive(Debug, Clone)]
pub struct DocumentSearchResult {
    /// Source document identifier (without chunk suffix)
    pub source_id: String,

    /// Workspace identifier
    pub workspace: String,

    /// Zero-based index of the best matching chunk
    pub best_chunk_index: usize,

    /// Distance of the best matching chunk
    pub best_chunk_distance: f32,

    /// Optional text excerpt from the best chunk
    pub excerpt: Option<String>,

    /// Total number of chunks in this document
    pub total_chunks: usize,

    /// Full node_id of the best chunk (includes chunk information)
    pub best_node_id: String,

    /// Revision of the best chunk
    pub best_revision: HLC,
}

impl DocumentSearchResult {
    /// Create from the best chunk result for a document.
    pub fn from_chunk_result(chunk: ChunkSearchResult) -> Self {
        Self {
            source_id: chunk.source_id,
            workspace: chunk.workspace,
            best_chunk_index: chunk.chunk_index,
            best_chunk_distance: chunk.distance,
            excerpt: chunk.excerpt,
            total_chunks: chunk.total_chunks,
            best_node_id: chunk.node_id,
            best_revision: chunk.revision,
        }
    }

    /// Convert distance to similarity score (0.0 = dissimilar, 1.0 = identical)
    pub fn similarity(&self) -> f32 {
        1.0 - (self.best_chunk_distance / 2.0).min(1.0)
    }
}

/// Parse chunk information from a node_id.
///
/// Expected format: `{source_id}#{chunk_index}`
/// If no `#` is found, treats the entire node_id as source_id with chunk_index = 0.
///
/// # Arguments
///
/// * `node_id` - Node identifier potentially containing chunk information
///
/// # Returns
///
/// Tuple of (source_id, chunk_index)
pub fn parse_chunk_id(node_id: &str) -> (String, usize) {
    if let Some(pos) = node_id.rfind('#') {
        let source_id = &node_id[..pos];
        let chunk_str = &node_id[pos + 1..];

        // Try to parse chunk index, default to 0 if parsing fails
        let chunk_index = chunk_str.parse::<usize>().unwrap_or(0);

        (source_id.to_string(), chunk_index)
    } else {
        // No chunk suffix, treat entire node_id as source_id
        (node_id.to_string(), 0)
    }
}

/// Deduplicate search results by source document, keeping the best chunk per document.
///
/// # Arguments
///
/// * `results` - Vector of search results (should be sorted by distance)
/// * `k` - Maximum number of documents to return
///
/// # Returns
///
/// Vector of deduplicated results, one per source document
pub fn deduplicate_by_document(results: Vec<SearchResult>, k: usize) -> Vec<SearchResult> {
    let mut seen_sources: HashMap<String, SearchResult> = HashMap::new();

    // Group by source_id, keeping the best (lowest distance) chunk per source
    for result in results {
        let (source_id, _chunk_index) = parse_chunk_id(&result.node_id);

        // Only insert if we haven't seen this source, or this chunk is better
        seen_sources
            .entry(source_id)
            .and_modify(|existing| {
                if result.distance < existing.distance {
                    *existing = result.clone();
                }
            })
            .or_insert(result);
    }

    // Collect and sort by distance
    let mut deduplicated: Vec<SearchResult> = seen_sources.into_values().collect();
    deduplicated.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit to k results
    deduplicated.truncate(k);
    deduplicated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chunk_id() {
        // With chunk suffix
        let (source, chunk) = parse_chunk_id("doc123#5");
        assert_eq!(source, "doc123");
        assert_eq!(chunk, 5);

        // Without chunk suffix
        let (source, chunk) = parse_chunk_id("doc123");
        assert_eq!(source, "doc123");
        assert_eq!(chunk, 0);

        // Multiple # characters (uses rightmost)
        let (source, chunk) = parse_chunk_id("path/to#doc#3");
        assert_eq!(source, "path/to#doc");
        assert_eq!(chunk, 3);

        // Invalid chunk index
        let (source, chunk) = parse_chunk_id("doc123#invalid");
        assert_eq!(source, "doc123");
        assert_eq!(chunk, 0);
    }

    #[test]
    fn test_deduplicate_by_document() {
        let results = vec![
            SearchResult::new("doc1#0".to_string(), "ws1".to_string(), HLC::new(1, 0), 0.1),
            SearchResult::new("doc1#1".to_string(), "ws1".to_string(), HLC::new(2, 0), 0.3),
            SearchResult::new("doc2#0".to_string(), "ws1".to_string(), HLC::new(3, 0), 0.2),
            SearchResult::new(
                "doc1#2".to_string(),
                "ws1".to_string(),
                HLC::new(4, 0),
                0.15,
            ),
            SearchResult::new(
                "doc3#0".to_string(),
                "ws1".to_string(),
                HLC::new(5, 0),
                0.25,
            ),
        ];

        let deduplicated = deduplicate_by_document(results, 10);

        // Should have 3 documents (doc1, doc2, doc3)
        assert_eq!(deduplicated.len(), 3);

        // doc1 should use chunk #0 (distance 0.1, best of 0.1, 0.15, 0.3)
        let doc1_result = deduplicated
            .iter()
            .find(|r| r.node_id.starts_with("doc1"))
            .unwrap();
        assert_eq!(doc1_result.node_id, "doc1#0");
        assert_eq!(doc1_result.distance, 0.1);

        // Results should be sorted by distance
        assert_eq!(deduplicated[0].distance, 0.1); // doc1
        assert_eq!(deduplicated[1].distance, 0.2); // doc2
        assert_eq!(deduplicated[2].distance, 0.25); // doc3
    }

    #[test]
    fn test_deduplicate_with_limit() {
        let results = vec![
            SearchResult::new("doc1#0".to_string(), "ws1".to_string(), HLC::new(1, 0), 0.1),
            SearchResult::new("doc2#0".to_string(), "ws1".to_string(), HLC::new(2, 0), 0.2),
            SearchResult::new("doc3#0".to_string(), "ws1".to_string(), HLC::new(3, 0), 0.3),
        ];

        let deduplicated = deduplicate_by_document(results, 2);

        // Should limit to 2 results
        assert_eq!(deduplicated.len(), 2);
        assert_eq!(deduplicated[0].node_id, "doc1#0");
        assert_eq!(deduplicated[1].node_id, "doc2#0");
    }

    #[test]
    fn test_scoring_config_default() {
        let config = ScoringConfig::default();
        assert_eq!(config.position_decay, 0.1);
        assert_eq!(config.first_chunk_boost, 1.2);
        assert_eq!(config.exact_match_boost, 1.0);
    }

    #[test]
    fn test_search_request_builder() {
        let request = SearchRequest::new(vec![1.0, 2.0, 3.0], 10)
            .with_mode(SearchMode::Chunks)
            .with_workspace("test".to_string())
            .with_max_distance(0.5)
            .with_scoring(ScoringConfig::default());

        assert_eq!(request.k, 10);
        assert_eq!(request.mode, SearchMode::Chunks);
        assert_eq!(request.workspace_filter, Some("test".to_string()));
        assert_eq!(request.max_distance, Some(0.5));
        assert!(request.scoring.is_some());
    }
}
