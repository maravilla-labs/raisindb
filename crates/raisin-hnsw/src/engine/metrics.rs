// SPDX-License-Identifier: BSL-1.1

//! Metrics for the HNSW vector search engine.
//!
//! Thread-safe atomic counters and histograms following the same pattern
//! as the replication metrics in `raisin-replication`.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Collected metrics for the HNSW vector search subsystem.
///
/// Exposed via management endpoints for monitoring dashboards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMetricsSnapshot {
    /// Total search requests
    pub search_count: u64,
    /// Total search results returned
    pub search_results_total: u64,
    /// Average search latency in milliseconds
    pub search_avg_ms: f64,
    /// P99 search latency in milliseconds
    pub search_p99_ms: f64,
    /// Cache hits (index found in LRU cache)
    pub cache_hits: u64,
    /// Cache misses (index loaded from disk)
    pub cache_misses: u64,
    /// Cache hit ratio (0.0-1.0)
    pub cache_hit_ratio: f64,
    /// Total embeddings added
    pub embeddings_added: u64,
    /// Total embeddings removed
    pub embeddings_removed: u64,
    /// Current number of loaded indexes
    pub indexes_loaded: u64,
}

/// Live metrics collector for HNSW operations.
///
/// Uses atomic operations for minimal overhead (<1%).
/// Call `snapshot()` to get a point-in-time view.
pub struct VectorMetrics {
    pub(crate) search_count: AtomicU64,
    pub(crate) search_results_total: AtomicU64,
    pub(crate) search_duration_samples: Arc<Mutex<Vec<u64>>>,
    pub(crate) search_duration_total_ms: AtomicU64,
    pub(crate) cache_hits: AtomicU64,
    pub(crate) cache_misses: AtomicU64,
    pub(crate) embeddings_added: AtomicU64,
    pub(crate) embeddings_removed: AtomicU64,
    pub(crate) indexes_loaded: AtomicU64,
    max_samples: usize,
    /// Simple counter for reservoir sampling (avoids rand dependency)
    sample_counter: AtomicUsize,
}

impl VectorMetrics {
    pub fn new() -> Self {
        Self {
            search_count: AtomicU64::new(0),
            search_results_total: AtomicU64::new(0),
            search_duration_samples: Arc::new(Mutex::new(Vec::with_capacity(1000))),
            search_duration_total_ms: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            embeddings_added: AtomicU64::new(0),
            embeddings_removed: AtomicU64::new(0),
            indexes_loaded: AtomicU64::new(0),
            max_samples: 1000,
            sample_counter: AtomicUsize::new(0),
        }
    }

    /// Record a search operation.
    pub fn record_search(&self, duration: Duration, result_count: usize) {
        self.search_count.fetch_add(1, Ordering::Relaxed);
        self.search_results_total
            .fetch_add(result_count as u64, Ordering::Relaxed);

        let ms = duration.as_millis() as u64;
        self.search_duration_total_ms.fetch_add(ms, Ordering::Relaxed);

        let mut samples = self.search_duration_samples.lock().unwrap();
        if samples.len() < self.max_samples {
            samples.push(ms);
        } else {
            // Simple deterministic reservoir sampling using a counter
            let idx = self.sample_counter.fetch_add(1, Ordering::Relaxed) % samples.len();
            samples[idx] = ms;
        }
    }

    /// Record a cache hit.
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss.
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an embedding addition.
    pub fn record_embedding_added(&self) {
        self.embeddings_added.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an embedding removal.
    pub fn record_embedding_removed(&self) {
        self.embeddings_removed.fetch_add(1, Ordering::Relaxed);
    }

    /// Take a point-in-time snapshot of all metrics.
    pub fn snapshot(&self) -> VectorMetricsSnapshot {
        let search_count = self.search_count.load(Ordering::Relaxed);
        let cache_hits = self.cache_hits.load(Ordering::Relaxed);
        let cache_misses = self.cache_misses.load(Ordering::Relaxed);
        let total_cache = cache_hits + cache_misses;

        let search_avg_ms = if search_count > 0 {
            self.search_duration_total_ms.load(Ordering::Relaxed) as f64 / search_count as f64
        } else {
            0.0
        };

        let search_p99_ms = {
            let samples = self.search_duration_samples.lock().unwrap();
            if samples.is_empty() {
                0.0
            } else {
                let mut sorted = samples.clone();
                sorted.sort_unstable();
                let idx = ((0.99 * sorted.len() as f64) as usize).min(sorted.len() - 1);
                sorted[idx] as f64
            }
        };

        VectorMetricsSnapshot {
            search_count,
            search_results_total: self.search_results_total.load(Ordering::Relaxed),
            search_avg_ms,
            search_p99_ms,
            cache_hits,
            cache_misses,
            cache_hit_ratio: if total_cache > 0 {
                cache_hits as f64 / total_cache as f64
            } else {
                0.0
            },
            embeddings_added: self.embeddings_added.load(Ordering::Relaxed),
            embeddings_removed: self.embeddings_removed.load(Ordering::Relaxed),
            indexes_loaded: self.indexes_loaded.load(Ordering::Relaxed),
        }
    }
}

impl Default for VectorMetrics {
    fn default() -> Self {
        Self::new()
    }
}
