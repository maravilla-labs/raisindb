//! Generic DashMap-based TTL cache.
//!
//! Provides a reusable, thread-safe in-memory cache with TTL-based expiration.
//! Built on `DashMap` for lock-free concurrent access across async tasks.
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_core::TtlCache;
//! use std::time::Duration;
//!
//! let cache: TtlCache<Vec<String>> = TtlCache::new(Duration::from_secs(60));
//!
//! // Put / get
//! cache.put("key", vec!["a".into()]);
//! assert_eq!(cache.get("key"), Some(vec!["a".into()]));
//!
//! // Async compute-on-miss
//! let val = cache.get_or_compute("key2", || async {
//!     Ok(vec!["computed".into()])
//! }).await?;
//! ```

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Cached entry with timestamp for TTL checking.
#[derive(Clone)]
struct CacheEntry<V> {
    value: V,
    cached_at: Instant,
}

/// Thread-safe generic cache with TTL-based expiration.
///
/// Uses `DashMap` for lock-free concurrent access across async tasks.
pub struct TtlCache<V: Clone + Send + Sync + 'static> {
    cache: DashMap<String, CacheEntry<V>>,
    ttl: Duration,
}

impl<V: Clone + Send + Sync + 'static> TtlCache<V> {
    /// Create a new cache with the specified TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: DashMap::new(),
            ttl,
        }
    }

    /// Create a cache with default 5-minute TTL.
    pub fn default_ttl() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Get a cached value if it exists and has not expired.
    pub fn get(&self, key: &str) -> Option<V> {
        self.cache.get(key).and_then(|entry| {
            if entry.cached_at.elapsed() < self.ttl {
                Some(entry.value.clone())
            } else {
                None
            }
        })
    }

    /// Store a value in the cache.
    pub fn put(&self, key: &str, value: V) {
        self.cache.insert(
            key.to_string(),
            CacheEntry {
                value,
                cached_at: Instant::now(),
            },
        );
    }

    /// Get a cached value, or compute and cache it on miss.
    pub async fn get_or_compute<F, Fut>(&self, key: &str, compute: F) -> raisin_error::Result<V>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = raisin_error::Result<V>>,
    {
        if let Some(val) = self.get(key) {
            return Ok(val);
        }

        let value = compute().await?;
        self.put(key, value.clone());
        Ok(value)
    }

    /// Remove a specific entry from the cache.
    pub fn invalidate(&self, key: &str) {
        self.cache.remove(key);
    }

    /// Remove multiple entries from the cache.
    pub fn invalidate_many(&self, keys: &[String]) {
        for key in keys {
            self.cache.remove(key.as_str());
        }
    }

    /// Remove all entries from the cache.
    pub fn invalidate_all(&self) {
        self.cache.clear();
    }

    /// Remove expired entries from the cache.
    pub fn cleanup_expired(&self) {
        self.cache
            .retain(|_, entry| entry.cached_at.elapsed() < self.ttl);
    }

    /// Get current cache statistics.
    pub fn stats(&self) -> CacheStats {
        let total = self.cache.len();
        let expired = self
            .cache
            .iter()
            .filter(|entry| entry.cached_at.elapsed() >= self.ttl)
            .count();

        CacheStats {
            total_entries: total,
            expired_entries: expired,
            valid_entries: total - expired,
        }
    }
}

impl<V: Clone + Send + Sync + 'static> Default for TtlCache<V> {
    fn default() -> Self {
        Self::default_ttl()
    }
}

/// Cache statistics for monitoring.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Total number of entries in cache.
    pub total_entries: usize,
    /// Number of expired entries (not yet cleaned up).
    pub expired_entries: usize,
    /// Number of valid (non-expired) entries.
    pub valid_entries: usize,
}

/// Shared cache handle that can be passed across services.
pub type SharedTtlCache<V> = Arc<TtlCache<V>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_and_get() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.put("key1", "value1".to_string());
        assert_eq!(cache.get("key1"), Some("value1".to_string()));
        assert_eq!(cache.get("missing"), None);
    }

    #[tokio::test]
    async fn test_expiration() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_millis(50));
        cache.put("key1", "value1".to_string());
        assert!(cache.get("key1").is_some());

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(cache.get("key1").is_none());
    }

    #[tokio::test]
    async fn test_get_or_compute() {
        let cache: TtlCache<Vec<String>> = TtlCache::new(Duration::from_secs(60));

        let val = cache
            .get_or_compute("key1", || async { Ok(vec!["computed".to_string()]) })
            .await
            .unwrap();
        assert_eq!(val, vec!["computed".to_string()]);

        // Second call should hit cache
        let mut called = false;
        let val = cache
            .get_or_compute("key1", || async {
                called = true;
                Ok(vec!["recomputed".to_string()])
            })
            .await
            .unwrap();
        assert_eq!(val, vec!["computed".to_string()]);
        assert!(!called);
    }

    #[test]
    fn test_invalidate() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.put("a", "1".to_string());
        cache.put("b", "2".to_string());

        cache.invalidate("a");
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
    }

    #[test]
    fn test_invalidate_all() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.put("a", "1".to_string());
        cache.put("b", "2".to_string());

        cache.invalidate_all();
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_stats() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.put("a", "1".to_string());
        cache.put("b", "2".to_string());

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.valid_entries, 2);
        assert_eq!(stats.expired_entries, 0);
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_millis(50));
        cache.put("a", "1".to_string());
        cache.put("b", "2".to_string());

        tokio::time::sleep(Duration::from_millis(100)).await;
        cache.put("c", "3".to_string()); // fresh entry

        cache.cleanup_expired();
        let stats = cache.stats();
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.valid_entries, 1);
    }
}
