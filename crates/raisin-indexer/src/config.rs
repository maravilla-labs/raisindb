// SPDX-License-Identifier: BSL-1.1

//! Configuration for index caching
//!
//! Provides configuration for index cache sizing to prevent memory exhaustion
//! in multi-tenant scenarios.

/// Configuration for index caching
///
/// Controls memory limits for different types of indexes to prevent
/// unbounded memory growth in multi-tenant deployments.
///
/// # Memory Sizing
///
/// - **Fulltext (Tantivy)**: ~30MB per index
/// - **Vector (HNSW)**: ~50MB per index (future)
///
/// # Multi-Tenant Scaling
///
/// With 500 tenants × 5 repos × 3 branches = 7,500 indexes:
/// - Development: 256MB fulltext → ~8 hot indexes cached
/// - Production: 1GB fulltext → ~34 hot indexes cached
#[derive(Debug, Clone)]
pub struct IndexCacheConfig {
    /// Fulltext (Tantivy) cache size in bytes
    pub fulltext_cache_size: usize,

    /// Vector (HNSW) cache size in bytes (for Phase 3)
    pub hnsw_cache_size: usize,
}

impl IndexCacheConfig {
    /// Development configuration with smaller cache sizes
    ///
    /// Suitable for local development and testing:
    /// - 256MB fulltext cache (~8 indexes)
    /// - 512MB vector cache (future)
    pub fn development() -> Self {
        Self {
            fulltext_cache_size: 256 * 1024 * 1024, // 256MB
            hnsw_cache_size: 512 * 1024 * 1024,     // 512MB
        }
    }

    /// Production configuration with larger cache sizes
    ///
    /// Suitable for production deployments:
    /// - 1GB fulltext cache (~34 indexes)
    /// - 2GB vector cache (future)
    pub fn production() -> Self {
        Self {
            fulltext_cache_size: 1024 * 1024 * 1024, // 1GB
            hnsw_cache_size: 2 * 1024 * 1024 * 1024, // 2GB
        }
    }

    /// Custom configuration with specified cache sizes
    ///
    /// # Arguments
    ///
    /// * `fulltext_cache_size` - Cache size in bytes for Tantivy indexes
    /// * `hnsw_cache_size` - Cache size in bytes for HNSW indexes
    pub fn custom(fulltext_cache_size: usize, hnsw_cache_size: usize) -> Self {
        Self {
            fulltext_cache_size,
            hnsw_cache_size,
        }
    }
}

impl Default for IndexCacheConfig {
    fn default() -> Self {
        Self::production()
    }
}
