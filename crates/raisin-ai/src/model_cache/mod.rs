//! Model caching with TTL support.
//!
//! Provides in-memory caching for model lists with time-to-live (TTL) to avoid
//! excessive API calls while keeping model lists fresh.

mod model_info;
mod profile;
#[cfg(test)]
mod tests;

pub use model_info::{ModelCapabilities, ModelInfo, SchemaTransformerType};
pub use profile::ModelProfile;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A cached entry with expiration time.
#[derive(Debug, Clone)]
struct CacheEntry {
    models: Vec<ModelInfo>,
    expires_at: Instant,
}

impl CacheEntry {
    fn new(models: Vec<ModelInfo>, ttl: Duration) -> Self {
        Self {
            models,
            expires_at: Instant::now() + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// In-memory cache for model lists with TTL.
#[derive(Debug, Clone)]
pub struct ModelCache {
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    default_ttl: Duration,
}

impl ModelCache {
    /// Creates a new model cache with the default TTL of 1 hour.
    pub fn new() -> Self {
        Self::with_ttl(Duration::from_secs(3600))
    }

    /// Creates a new model cache with a custom TTL.
    pub fn with_ttl(default_ttl: Duration) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl,
        }
    }

    /// Gets cached models for a provider, if available and not expired.
    pub async fn get(&self, provider_name: &str) -> Option<Vec<ModelInfo>> {
        let cache = self.cache.read().await;

        cache.get(provider_name).and_then(|entry| {
            if entry.is_expired() {
                None
            } else {
                Some(entry.models.clone())
            }
        })
    }

    /// Stores models in the cache with the default TTL.
    pub async fn put(&self, provider_name: impl Into<String>, models: Vec<ModelInfo>) {
        self.put_with_ttl(provider_name, models, self.default_ttl)
            .await;
    }

    /// Stores models in the cache with a custom TTL.
    pub async fn put_with_ttl(
        &self,
        provider_name: impl Into<String>,
        models: Vec<ModelInfo>,
        ttl: Duration,
    ) {
        let mut cache = self.cache.write().await;
        cache.insert(provider_name.into(), CacheEntry::new(models, ttl));
    }

    /// Invalidates the cache for a specific provider.
    pub async fn invalidate(&self, provider_name: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(provider_name);
    }

    /// Clears all cached entries.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Removes expired entries from the cache.
    pub async fn cleanup(&self) {
        let mut cache = self.cache.write().await;
        cache.retain(|_, entry| !entry.is_expired());
    }
}

impl Default for ModelCache {
    fn default() -> Self {
        Self::new()
    }
}
