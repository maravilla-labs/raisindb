// SPDX-License-Identifier: BSL-1.1

//! HNSW indexing engine with LRU cache and persistence.
//!
//! This module provides the main engine that manages multiple HNSW indexes
//! across tenants, repositories, and branches with memory-bounded caching.
//!
//! Uses full HLC (Hybrid Logical Clock) with 16-byte encoding for revision tracking
//! to preserve both timestamp and counter components for proper distributed consistency.

mod indexing;
mod lifecycle;
pub mod metrics;
mod search;

#[cfg(test)]
mod tests;

use crate::index::HnswIndex;
use crate::types::DistanceMetric;
use moka::sync::Cache;
use raisin_error::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// HNSW indexing engine with LRU cache.
///
/// This engine manages multiple HNSW indexes (one per tenant/repo/branch)
/// with automatic eviction based on memory usage.
///
/// # Features
///
/// - **LRU Eviction**: Automatically evicts least-recently-used indexes
/// - **Dirty Tracking**: Tracks which indexes have unsaved changes
/// - **Periodic Snapshots**: Background task saves dirty indexes every 60s
/// - **Graceful Shutdown**: Ensures all dirty indexes are saved on shutdown
pub struct HnswIndexingEngine {
    /// Base directory for index files
    base_path: PathBuf,

    /// LRU cache of loaded indexes
    index_cache: Cache<String, Arc<RwLock<HnswIndex>>>,

    /// Set of dirty index keys (need to be saved)
    dirty_indexes: Arc<RwLock<HashSet<String>>>,

    /// Vector dimensions (must be consistent across all indexes)
    dimensions: usize,

    /// Default distance metric for new indexes
    distance_metric: DistanceMetric,

    /// Observability metrics
    metrics: Arc<metrics::VectorMetrics>,
}

impl HnswIndexingEngine {
    /// Create a new HNSW indexing engine.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Directory where index files will be stored
    /// * `cache_size` - Maximum cache size in bytes
    /// * `dimensions` - Vector dimensionality (e.g., 1536 for OpenAI)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use raisin_hnsw::HnswIndexingEngine;
    /// use std::path::PathBuf;
    ///
    /// let engine = HnswIndexingEngine::new(
    ///     PathBuf::from("./.data/hnsw"),
    ///     2 * 1024 * 1024 * 1024,  // 2GB cache
    ///     1536                      // OpenAI dimensions
    /// )?;
    /// ```
    pub fn new(base_path: PathBuf, cache_size: usize, dimensions: usize) -> Result<Self> {
        Self::with_metric(base_path, cache_size, dimensions, DistanceMetric::default())
    }

    /// Create a new HNSW indexing engine with a specific distance metric.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Directory where index files will be stored
    /// * `cache_size` - Maximum cache size in bytes
    /// * `dimensions` - Vector dimensionality (e.g., 1536 for OpenAI)
    /// * `distance_metric` - Distance metric for new indexes
    pub fn with_metric(
        base_path: PathBuf,
        cache_size: usize,
        dimensions: usize,
        distance_metric: DistanceMetric,
    ) -> Result<Self> {
        let index_cache = Cache::builder()
            .weigher(|_key: &String, index: &Arc<RwLock<HnswIndex>>| -> u32 {
                let index_guard = index.read().unwrap();
                let size = index_guard.estimated_memory_bytes();
                (size as u64).min(u32::MAX as u64) as u32
            })
            .max_capacity(cache_size as u64)
            .eviction_listener(|key, _value, cause| {
                tracing::info!("Evicted HNSW index: {} (cause: {:?})", key, cause);
            })
            .build();

        Ok(Self {
            base_path,
            index_cache,
            dirty_indexes: Arc::new(RwLock::new(HashSet::new())),
            dimensions,
            distance_metric,
            metrics: Arc::new(metrics::VectorMetrics::new()),
        })
    }

    /// Get the default distance metric for this engine.
    pub fn distance_metric(&self) -> DistanceMetric {
        self.distance_metric
    }

    /// Get a snapshot of vector search metrics.
    pub fn metrics(&self) -> metrics::VectorMetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Get or load an HNSW index for a specific context.
    ///
    /// If the index is in cache, returns it immediately.
    /// Otherwise, loads from disk or creates a new one.
    fn get_or_load_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Arc<RwLock<HnswIndex>>> {
        let key = self.make_key(tenant_id, repo_id, branch);

        // Check cache first
        if let Some(index) = self.index_cache.get(&key) {
            self.metrics.record_cache_hit();
            return Ok(index);
        }

        self.metrics.record_cache_miss();

        // Load from disk or create new
        let path = self.get_index_path(&key);
        let index = if path.exists() {
            HnswIndex::view_from_file(&path)?
        } else {
            HnswIndex::with_metric(self.dimensions, self.distance_metric)
        };

        let index_arc = Arc::new(RwLock::new(index));

        // Insert into cache
        self.index_cache.insert(key, Arc::clone(&index_arc));

        Ok(index_arc)
    }

    /// Make cache key from context.
    ///
    /// Note: workspace_id is NOT part of the key anymore.
    /// Each index covers ALL workspaces for a tenant/repo/branch.
    /// Workspace filtering happens at search time.
    fn make_key(&self, tenant_id: &str, repo_id: &str, branch: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, branch)
    }

    /// Get file path for an index.
    fn get_index_path(&self, key: &str) -> PathBuf {
        let mut path = self.base_path.clone();
        for part in key.split('/') {
            path.push(part);
        }
        path.with_extension("hnsw")
    }

    /// Get index statistics.
    ///
    /// Note: Returns stats for the entire index (all workspaces combined).
    pub fn stats(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<IndexStats> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch)?;
        let index = index_arc.read().unwrap();

        Ok(IndexStats {
            count: index.len(),
            dimensions: self.dimensions,
            memory_bytes: index.estimated_memory_bytes(),
        })
    }
}

/// Index statistics.
#[derive(Debug, Clone)]
pub struct IndexStats {
    /// Number of vectors in the index
    pub count: usize,

    /// Vector dimensions
    pub dimensions: usize,

    /// Estimated memory usage in bytes
    pub memory_bytes: usize,
}
