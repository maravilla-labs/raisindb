//! Permission caching for efficient RLS enforcement.
//!
//! This module provides in-memory caching of resolved permissions with TTL-based expiration.
//! Caching avoids repeated database queries for permission resolution on every request.
//!
//! Internally delegates to [`TtlCache`](super::ttl_cache::TtlCache).
//!
//! # Cache Invalidation
//!
//! The cache should be invalidated when:
//! - A user's direct roles or groups change
//! - A group's roles change
//! - A role's permissions or inherits change
//!
//! # Usage
//!
//! ```rust,ignore
//! let cache = PermissionCache::new(Duration::from_secs(300)); // 5 minute TTL
//!
//! // Get or compute permissions
//! let permissions = cache.get_or_compute("user123", || async {
//!     permission_service.resolve_for_user("user123").await
//! }).await?;
//!
//! // Invalidate on change
//! cache.invalidate("user123");
//! ```

use raisin_models::permissions::ResolvedPermissions;
use std::sync::Arc;
use std::time::Duration;

// Re-export CacheStats from ttl_cache for backward compatibility.
pub use super::ttl_cache::CacheStats;
use super::ttl_cache::TtlCache;

/// Thread-safe permission cache with TTL-based expiration.
///
/// Wraps a generic [`TtlCache`] specialised on [`ResolvedPermissions`].
pub struct PermissionCache {
    inner: TtlCache<ResolvedPermissions>,
}

impl PermissionCache {
    /// Create a new permission cache with the specified TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: TtlCache::new(ttl),
        }
    }

    /// Create a permission cache with default 5-minute TTL.
    pub fn default_ttl() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Get cached permissions for a user, or compute and cache them.
    pub async fn get_or_compute<F, Fut>(
        &self,
        user_id: &str,
        compute: F,
    ) -> raisin_error::Result<ResolvedPermissions>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = raisin_error::Result<ResolvedPermissions>>,
    {
        // Check cache first
        if let Some(permissions) = self.inner.get(user_id) {
            tracing::debug!(user_id = user_id, "Permission cache hit");
            return Ok(permissions);
        }

        // Compute permissions
        tracing::debug!(user_id = user_id, "Permission cache miss, computing");
        let permissions = compute().await?;

        // Cache the result
        self.inner.put(user_id, permissions.clone());
        Ok(permissions)
    }

    /// Get cached permissions if they exist and are valid.
    pub fn get(&self, user_id: &str) -> Option<ResolvedPermissions> {
        self.inner.get(user_id)
    }

    /// Cache permissions for a user.
    pub fn put(&self, user_id: &str, permissions: ResolvedPermissions) {
        self.inner.put(user_id, permissions);
    }

    /// Invalidate cached permissions for a specific user.
    pub fn invalidate(&self, user_id: &str) {
        if self.inner.get(user_id).is_some() {
            self.inner.invalidate(user_id);
            tracing::debug!(user_id = user_id, "Permission cache invalidated");
        }
    }

    /// Invalidate all cached permissions for a list of users.
    pub fn invalidate_many(&self, user_ids: &[String]) {
        self.inner.invalidate_many(user_ids);
        tracing::debug!(
            count = user_ids.len(),
            "Permission cache invalidated for multiple users"
        );
    }

    /// Invalidate all cached permissions.
    pub fn invalidate_all(&self) {
        let count = self.inner.stats().total_entries;
        self.inner.invalidate_all();
        tracing::info!(count = count, "Permission cache cleared");
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
                "Cleaned up expired permission cache entries"
            );
        }
    }

    /// Get current cache statistics.
    pub fn stats(&self) -> CacheStats {
        self.inner.stats()
    }
}

impl Default for PermissionCache {
    fn default() -> Self {
        Self::default_ttl()
    }
}

/// Shared permission cache that can be passed across services.
pub type SharedPermissionCache = Arc<PermissionCache>;

/// Create a new shared permission cache.
pub fn new_shared_cache(ttl: Duration) -> SharedPermissionCache {
    Arc::new(PermissionCache::new(ttl))
}

/// Create a new shared permission cache with default TTL.
pub fn new_shared_cache_default() -> SharedPermissionCache {
    Arc::new(PermissionCache::default_ttl())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_permissions(user_id: &str) -> ResolvedPermissions {
        ResolvedPermissions {
            user_id: user_id.to_string(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions: vec![],
            is_system_admin: false,
            resolved_at: None,
        }
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let cache = PermissionCache::new(Duration::from_secs(60));

        // First call computes
        let result = cache
            .get_or_compute("user1", || async { Ok(make_permissions("user1")) })
            .await
            .unwrap();
        assert_eq!(result.user_id, "user1");

        // Second call hits cache
        let mut called = false;
        let result = cache
            .get_or_compute("user1", || async {
                called = true;
                Ok(make_permissions("user1"))
            })
            .await
            .unwrap();
        assert_eq!(result.user_id, "user1");
        assert!(!called, "Should not have computed again");
    }

    #[tokio::test]
    async fn test_cache_invalidation() {
        let cache = PermissionCache::new(Duration::from_secs(60));

        // Cache a value
        cache.put("user1", make_permissions("user1"));
        assert!(cache.get("user1").is_some());

        // Invalidate
        cache.invalidate("user1");
        assert!(cache.get("user1").is_none());
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache = PermissionCache::new(Duration::from_millis(50));

        // Cache a value
        cache.put("user1", make_permissions("user1"));
        assert!(cache.get("user1").is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(cache.get("user1").is_none());
    }

    #[test]
    fn test_cache_stats() {
        let cache = PermissionCache::new(Duration::from_secs(60));

        cache.put("user1", make_permissions("user1"));
        cache.put("user2", make_permissions("user2"));

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.valid_entries, 2);
        assert_eq!(stats.expired_entries, 0);
    }
}
