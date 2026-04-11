// SPDX-License-Identifier: BSL-1.1

//! HNSW vector search engine for RaisinDB.
//!
//! This crate provides fast approximate nearest neighbor (ANN) search using the
//! Hierarchical Navigable Small World (HNSW) graph algorithm with **cosine distance**
//! for normalized OpenAI embeddings.
//!
//! # Architecture
//!
//! - **Memory-Bounded**: Uses Moka LRU cache to limit memory usage
//! - **Multi-Tenant**: Separate indexes per tenant/repo/branch
//! - **Persistent**: Periodic snapshots to disk with dirty tracking
//! - **Crash-Safe**: Graceful shutdown ensures all dirty indexes are saved
//! - **Cosine Distance**: Optimized for normalized vectors (OpenAI embeddings)
//!
//! # Key Features
//!
//! - O(log n) approximate nearest neighbor search
//! - Cosine distance for better semantic similarity (vs L2 in high dimensions)
//! - LRU eviction for memory management (configurable size)
//! - Background snapshot task (60s interval)
//! - Branch copy operations (Git-like semantics)
//! - Rebuild from RocksDB embeddings CF
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_hnsw::HnswIndexingEngine;
//!
//! // Create engine with 2GB cache
//! let engine = HnswIndexingEngine::new(
//!     PathBuf::from("./.data/hnsw"),
//!     2 * 1024 * 1024 * 1024
//! )?;
//!
//! // Start periodic snapshot task
//! let snapshot_handle = engine.start_snapshot_task();
//!
//! // Add embedding
//! engine.add_embedding("tenant1", "repo1", "main", "ws1", "node1", 42, embedding)?;
//!
//! // Search for similar vectors
//! let results = engine.search("tenant1", "repo1", "main", "ws1", &query_vector, 10)?;
//!
//! // Graceful shutdown
//! engine.shutdown().await?;
//! snapshot_handle.abort();
//! ```

pub mod engine;
pub mod excerpt;
pub mod index;
mod migration;
mod persistence;
pub mod types;

pub use engine::metrics::VectorMetricsSnapshot;
pub use engine::HnswIndexingEngine;
pub use excerpt::{ExcerptFetcher, ExcerptRequest};
pub use index::HnswIndex;
pub use types::{
    ChunkSearchResult, DistanceMetric, DocumentSearchResult, HnswParams, QuantizationType,
    ScoringConfig, SearchMode, SearchRequest, SearchResult, VectorPoint,
};

// Re-export key types
pub use raisin_error::Result;

/// Normalize a vector to unit length (L2 norm = 1.0).
///
/// **CRITICAL**: This normalization is required for cosine distance to work correctly.
/// OpenAI embeddings are pre-normalized, but we normalize again to ensure consistency.
///
/// This function normalizes vectors so that cosine similarity can be computed
/// efficiently using just the dot product:
///
///   cosine_similarity(a, b) = dot(a, b)  (when both are normalized)
///   cosine_distance = 1 - dot(a, b)
///
/// Distance interpretation (cosine distance):
/// - Distance 0.0 = identical vectors (cosine sim = 1.0)
/// - Distance 0.2-0.4 = semantically similar (cosine sim 0.8-0.6)
/// - Distance 0.4-0.6 = weakly related (cosine sim 0.6-0.4)
/// - Distance > 0.6 = not related (cosine sim < 0.4)
///
/// # Arguments
///
/// * `vector` - Input vector of any length
///
/// # Returns
///
/// A new vector with the same direction but unit length (magnitude = 1.0).
/// If the input is a zero vector, returns it unchanged.
///
/// # Example
///
/// ```
/// let v = vec![3.0, 4.0];
/// let normalized = raisin_hnsw::normalize_vector(&v);
/// // normalized ≈ [0.6, 0.8]  (magnitude = 1.0)
/// ```
pub fn normalize_vector(vector: &[f32]) -> Vec<f32> {
    // Calculate L2 norm (magnitude)
    let magnitude = vector.iter().map(|x| x * x).sum::<f32>().sqrt();

    // Return as-is if zero vector (avoid division by zero)
    if magnitude == 0.0 || !magnitude.is_finite() {
        return vector.to_vec();
    }

    // Normalize: divide each component by magnitude
    vector.iter().map(|x| x / magnitude).collect()
}
