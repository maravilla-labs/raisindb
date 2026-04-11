//! Schema statistics cache for data-driven query plan selectivity estimation.
//!
//! Caches per-branch schema metadata (NodeType and Archetype counts) to avoid
//! repeated storage lookups during query planning. Follows the same TTL-based
//! cache pattern as [`PermissionCache`].
//!
//! # Cache Key
//!
//! Keys use the format `"{tenant_id}:{repo_id}:{branch}"` to scope stats per branch.
//!
//! # Usage
//!
//! ```rust,ignore
//! let cache = SchemaStatsCache::new(Duration::from_secs(300)); // 5 minute TTL
//!
//! // Get or compute stats
//! let stats = cache.get_or_compute("tenant:repo:main", || async {
//!     Ok(SchemaStats { node_type_count: 12, archetype_count: 5 })
//! }).await?;
//!
//! // Invalidate on schema change
//! cache.invalidate("tenant:repo:main");
//! ```

use std::sync::Arc;
use std::time::Duration;

// Re-export CacheStats from ttl_cache for convenience.
pub use super::ttl_cache::CacheStats;
use super::ttl_cache::TtlCache;

/// Cached schema statistics for a single branch scope.
#[derive(Debug, Clone)]
pub struct SchemaStats {
    /// Number of distinct NodeType definitions on this branch.
    pub node_type_count: usize,
    /// Number of distinct Archetype definitions on this branch.
    pub archetype_count: usize,
}

/// TTL-based cache for schema statistics, keyed by branch scope string
/// (`"{tenant_id}:{repo_id}:{branch}"`).
pub struct SchemaStatsCache {
    inner: TtlCache<SchemaStats>,
}

impl SchemaStatsCache {
    /// Create a new schema stats cache with the specified TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: TtlCache::new(ttl),
        }
    }

    /// Create a schema stats cache with default 5-minute TTL.
    pub fn default_ttl() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Get cached schema stats if they exist and are valid.
    pub fn get(&self, scope_key: &str) -> Option<SchemaStats> {
        self.inner.get(scope_key)
    }

    /// Cache schema stats for a branch scope.
    pub fn put(&self, scope_key: &str, stats: SchemaStats) {
        self.inner.put(scope_key, stats);
    }

    /// Get cached schema stats, or compute and cache them on miss.
    pub async fn get_or_compute<F, Fut>(
        &self,
        scope_key: &str,
        compute: F,
    ) -> raisin_error::Result<SchemaStats>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = raisin_error::Result<SchemaStats>>,
    {
        if let Some(stats) = self.inner.get(scope_key) {
            tracing::debug!(scope_key = scope_key, "Schema stats cache hit");
            return Ok(stats);
        }

        tracing::debug!(scope_key = scope_key, "Schema stats cache miss, computing");
        let stats = compute().await?;

        self.inner.put(scope_key, stats.clone());
        Ok(stats)
    }

    /// Invalidate cached schema stats for a specific branch scope.
    pub fn invalidate(&self, scope_key: &str) {
        if self.inner.get(scope_key).is_some() {
            self.inner.invalidate(scope_key);
            tracing::debug!(scope_key = scope_key, "Schema stats cache invalidated");
        }
    }

    /// Invalidate all cached schema stats.
    pub fn invalidate_all(&self) {
        let count = self.inner.stats().total_entries;
        self.inner.invalidate_all();
        tracing::info!(count = count, "Schema stats cache cleared");
    }

    /// Remove expired entries from the cache.
    pub fn cleanup_expired(&self) {
        let before = self.inner.stats().total_entries;
        self.inner.cleanup_expired();
        let after = self.inner.stats().total_entries;
        let removed = before - after;
        if removed > 0 {
            tracing::debug!(
                removed = removed,
                "Cleaned up expired schema stats cache entries"
            );
        }
    }

    /// Get current cache statistics.
    pub fn stats(&self) -> CacheStats {
        self.inner.stats()
    }
}

impl Default for SchemaStatsCache {
    fn default() -> Self {
        Self::default_ttl()
    }
}

/// Shared schema stats cache that can be passed across services.
pub type SharedSchemaStatsCache = Arc<SchemaStatsCache>;

/// Create a new shared schema stats cache.
pub fn new_shared_cache(ttl: Duration) -> SharedSchemaStatsCache {
    Arc::new(SchemaStatsCache::new(ttl))
}

/// Create a new shared schema stats cache with default TTL.
pub fn new_shared_cache_default() -> SharedSchemaStatsCache {
    Arc::new(SchemaStatsCache::default_ttl())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_stats(node_types: usize, archetypes: usize) -> SchemaStats {
        SchemaStats {
            node_type_count: node_types,
            archetype_count: archetypes,
        }
    }

    #[test]
    fn test_cache_put_and_get() {
        let cache = SchemaStatsCache::new(Duration::from_secs(60));

        cache.put("tenant:repo:main", make_stats(10, 3));
        let stats = cache.get("tenant:repo:main").unwrap();
        assert_eq!(stats.node_type_count, 10);
        assert_eq!(stats.archetype_count, 3);
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let cache = SchemaStatsCache::new(Duration::from_secs(60));

        assert!(cache.get("unknown:scope:key").is_none());
    }

    #[test]
    fn test_cache_invalidate() {
        let cache = SchemaStatsCache::new(Duration::from_secs(60));

        cache.put("tenant:repo:main", make_stats(5, 2));
        assert!(cache.get("tenant:repo:main").is_some());

        cache.invalidate("tenant:repo:main");
        assert!(cache.get("tenant:repo:main").is_none());
    }

    #[tokio::test]
    async fn test_cache_ttl_expiry() {
        let cache = SchemaStatsCache::new(Duration::from_millis(10));

        cache.put("tenant:repo:main", make_stats(7, 1));
        assert!(cache.get("tenant:repo:main").is_some());

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(cache.get("tenant:repo:main").is_none());
    }
}
