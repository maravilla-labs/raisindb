//! In-memory LRU cache layer for graph algorithm results
//!
//! Provides a fast lookup layer in front of RocksDB's GRAPH_CACHE column family.
//! Uses per-config LRU caches with DashMap for concurrent access.

use super::types::GraphCacheValue;
use dashmap::DashMap;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;

/// Default maximum entries per config cache
const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// In-memory LRU cache layer for graph algorithm results.
///
/// Each graph algorithm config has its own LRU cache to prevent
/// one algorithm's hot data from evicting another's.
pub struct GraphCacheLayer {
    /// Per-config LRU caches
    /// Key: config_id, Value: LRU cache of node_id -> GraphCacheValue
    caches: DashMap<String, Mutex<LruCache<String, GraphCacheValue>>>,
    /// Maximum entries per cache
    max_entries: NonZeroUsize,
}

impl GraphCacheLayer {
    /// Create a new cache layer with default settings
    pub fn new() -> Self {
        Self::with_max_entries(DEFAULT_MAX_ENTRIES)
    }

    /// Create a new cache layer with custom max entries per config
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self {
            caches: DashMap::new(),
            max_entries: NonZeroUsize::new(max_entries).unwrap_or(NonZeroUsize::new(1).unwrap()),
        }
    }

    /// Get a cached value for a node
    ///
    /// Returns `None` if not in cache or if the cache entry has expired.
    pub fn get(&self, config_id: &str, node_id: &str) -> Option<GraphCacheValue> {
        let cache_ref = self.caches.get(config_id)?;
        let mut cache = cache_ref.lock().ok()?;

        cache.get(node_id).and_then(|value| {
            if value.is_expired() {
                None
            } else {
                Some(value.clone())
            }
        })
    }

    /// Get multiple cached values for nodes (batch lookup)
    ///
    /// Returns a vector with `Some(value)` for cache hits and `None` for misses.
    pub fn get_batch(&self, config_id: &str, node_ids: &[&str]) -> Vec<Option<GraphCacheValue>> {
        let cache_ref = match self.caches.get(config_id) {
            Some(c) => c,
            None => return vec![None; node_ids.len()],
        };

        let mut cache = match cache_ref.lock() {
            Ok(c) => c,
            Err(_) => return vec![None; node_ids.len()],
        };

        node_ids
            .iter()
            .map(|node_id| {
                cache.get(*node_id).and_then(|value| {
                    if value.is_expired() {
                        None
                    } else {
                        Some(value.clone())
                    }
                })
            })
            .collect()
    }

    /// Put a value in the cache
    pub fn put(&self, config_id: &str, node_id: &str, value: GraphCacheValue) {
        // Ensure the config cache exists
        if !self.caches.contains_key(config_id) {
            self.caches.insert(
                config_id.to_string(),
                Mutex::new(LruCache::new(self.max_entries)),
            );
        }

        // Now get a reference and insert
        if let Some(cache_ref) = self.caches.get(config_id) {
            if let Ok(mut cache) = cache_ref.lock() {
                cache.put(node_id.to_string(), value);
            }
        }
    }

    /// Put multiple values in the cache (batch insert)
    pub fn put_batch(&self, config_id: &str, entries: Vec<(String, GraphCacheValue)>) {
        // Ensure the config cache exists
        if !self.caches.contains_key(config_id) {
            self.caches.insert(
                config_id.to_string(),
                Mutex::new(LruCache::new(self.max_entries)),
            );
        }

        // Now get a reference and insert all entries
        if let Some(cache_ref) = self.caches.get(config_id) {
            if let Ok(mut cache) = cache_ref.lock() {
                for (node_id, value) in entries {
                    cache.put(node_id, value);
                }
            }
        }
    }

    /// Invalidate all cached values for a config
    pub fn invalidate(&self, config_id: &str) {
        self.caches.remove(config_id);
    }

    /// Invalidate specific nodes from a config's cache
    pub fn invalidate_nodes(&self, config_id: &str, node_ids: &[&str]) {
        if let Some(cache_ref) = self.caches.get(config_id) {
            if let Ok(mut cache) = cache_ref.lock() {
                for node_id in node_ids {
                    cache.pop(*node_id);
                }
            }
        }
    }

    /// Clear all caches
    pub fn clear(&self) {
        self.caches.clear();
    }

    /// Get the number of configs with active caches
    pub fn config_count(&self) -> usize {
        self.caches.len()
    }

    /// Get the total number of cached entries across all configs
    pub fn total_entries(&self) -> usize {
        self.caches
            .iter()
            .filter_map(|entry| entry.value().lock().ok().map(|c| c.len()))
            .sum()
    }

    /// Get stats for a specific config's cache
    pub fn config_stats(&self, config_id: &str) -> Option<CacheStats> {
        let cache_ref = self.caches.get(config_id)?;
        let cache = cache_ref.lock().ok()?;

        Some(CacheStats {
            entries: cache.len(),
            capacity: self.max_entries.get(),
        })
    }
}

impl Default for GraphCacheLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for a single config's cache
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Number of entries currently in the cache
    pub entries: usize,
    /// Maximum capacity of the cache
    pub capacity: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::CachedValue;

    fn make_cache_value(float_val: f64) -> GraphCacheValue {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        GraphCacheValue {
            value: CachedValue::Float(float_val),
            computed_at: now,
            expires_at: now + 60_000, // 1 minute from now
            source_revision: "abc123".to_string(),
            config_revision: "v1".to_string(),
        }
    }

    fn make_expired_cache_value(float_val: f64) -> GraphCacheValue {
        GraphCacheValue {
            value: CachedValue::Float(float_val),
            computed_at: 1000,
            expires_at: 2000, // Already expired
            source_revision: "abc123".to_string(),
            config_revision: "v1".to_string(),
        }
    }

    #[test]
    fn test_put_and_get() {
        let cache = GraphCacheLayer::new();

        cache.put("pagerank-social", "user123", make_cache_value(0.85));

        let result = cache.get("pagerank-social", "user123");
        assert!(result.is_some());
        assert_eq!(result.unwrap().value.as_float(), Some(0.85));
    }

    #[test]
    fn test_get_nonexistent() {
        let cache = GraphCacheLayer::new();

        assert!(cache.get("pagerank-social", "user123").is_none());
        assert!(cache.get("nonexistent-config", "user123").is_none());
    }

    #[test]
    fn test_expired_entry_returns_none() {
        let cache = GraphCacheLayer::new();

        cache.put("pagerank-social", "user123", make_expired_cache_value(0.85));

        assert!(cache.get("pagerank-social", "user123").is_none());
    }

    #[test]
    fn test_invalidate_config() {
        let cache = GraphCacheLayer::new();

        cache.put("pagerank-social", "user123", make_cache_value(0.85));
        cache.put("pagerank-social", "user456", make_cache_value(0.75));

        cache.invalidate("pagerank-social");

        assert!(cache.get("pagerank-social", "user123").is_none());
        assert!(cache.get("pagerank-social", "user456").is_none());
    }

    #[test]
    fn test_invalidate_nodes() {
        let cache = GraphCacheLayer::new();

        cache.put("pagerank-social", "user123", make_cache_value(0.85));
        cache.put("pagerank-social", "user456", make_cache_value(0.75));
        cache.put("pagerank-social", "user789", make_cache_value(0.65));

        cache.invalidate_nodes("pagerank-social", &["user123", "user456"]);

        assert!(cache.get("pagerank-social", "user123").is_none());
        assert!(cache.get("pagerank-social", "user456").is_none());
        assert!(cache.get("pagerank-social", "user789").is_some());
    }

    #[test]
    fn test_batch_operations() {
        let cache = GraphCacheLayer::new();

        let entries = vec![
            ("user1".to_string(), make_cache_value(0.9)),
            ("user2".to_string(), make_cache_value(0.8)),
            ("user3".to_string(), make_cache_value(0.7)),
        ];

        cache.put_batch("pagerank", entries);

        let results = cache.get_batch("pagerank", &["user1", "user2", "user3", "user4"]);

        assert_eq!(results.len(), 4);
        assert!(results[0].is_some());
        assert!(results[1].is_some());
        assert!(results[2].is_some());
        assert!(results[3].is_none()); // user4 not in cache
    }

    #[test]
    fn test_stats() {
        let cache = GraphCacheLayer::new();

        cache.put("config1", "user1", make_cache_value(0.9));
        cache.put("config1", "user2", make_cache_value(0.8));
        cache.put("config2", "user1", make_cache_value(0.7));

        assert_eq!(cache.config_count(), 2);
        assert_eq!(cache.total_entries(), 3);

        let stats = cache.config_stats("config1").unwrap();
        assert_eq!(stats.entries, 2);
    }

    #[test]
    fn test_lru_eviction() {
        let cache = GraphCacheLayer::with_max_entries(2);

        cache.put("config", "user1", make_cache_value(0.9));
        cache.put("config", "user2", make_cache_value(0.8));
        cache.put("config", "user3", make_cache_value(0.7)); // Should evict user1

        assert!(cache.get("config", "user1").is_none()); // Evicted
        assert!(cache.get("config", "user2").is_some());
        assert!(cache.get("config", "user3").is_some());
    }
}
