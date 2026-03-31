// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! `PermissionCache` - Thread-safe LRU cache for workspace permissions.

use lru::LruCache;
use raisin_error::Result;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use super::types::{CacheKey, CachedPermissions};

/// In-memory LRU cache for workspace permissions.
///
/// Thread-safe cache that stores workspace permissions with TTL-based expiration
/// and LRU eviction policy. Designed for high-throughput read scenarios with
/// occasional writes for invalidation.
///
/// # Thread Safety
///
/// Uses `RwLock` to allow multiple concurrent readers while ensuring exclusive
/// access for writers. This is optimal for caches where reads vastly outnumber writes.
///
/// # Capacity and Eviction
///
/// When the cache reaches capacity, the least recently used entry is evicted.
/// The capacity should be set based on:
/// - Expected number of active sessions
/// - Average workspaces per user
/// - Available memory
///
/// Rule of thumb: `capacity = expected_active_sessions * avg_workspaces_per_user * 1.5`
pub struct PermissionCache {
    cache: Arc<RwLock<LruCache<CacheKey, CachedPermissions>>>,
    ttl: Duration,
}

impl PermissionCache {
    /// Create a new permission cache with the specified capacity and TTL.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of entries to store (LRU eviction when exceeded)
    /// * `ttl` - Time-to-live for cached entries
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity =
            NonZeroUsize::new(capacity).expect("Cache capacity must be greater than zero");

        Self {
            cache: Arc::new(RwLock::new(LruCache::new(capacity))),
            ttl,
        }
    }

    /// Get cached permissions or resolve using the provided resolver function.
    ///
    /// Implements a cache-aside pattern:
    /// 1. Check if the key exists in cache and is not expired
    /// 2. If found and valid, return cached value
    /// 3. If not found or expired, call the resolver function
    /// 4. Cache the resolved value and return it
    pub async fn get_or_resolve<F, Fut>(
        &self,
        key: CacheKey,
        resolver: F,
    ) -> Result<CachedPermissions>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<CachedPermissions>>,
    {
        // Try to get from cache first (write lock for LRU promotion)
        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&key) {
                if !cached.is_expired(self.ttl) {
                    return Ok(cached.clone());
                }
                // Expired - remove it
                cache.pop(&key);
            }
        }

        // Cache miss or expired - resolve
        let permissions = resolver().await?;

        // Store in cache (write lock)
        {
            let mut cache = self.cache.write().await;
            cache.put(key, permissions.clone());
        }

        Ok(permissions)
    }

    /// Get permissions if cached and not expired.
    ///
    /// Returns `None` if the key is not in the cache or if the cached value has expired.
    pub async fn get(&self, key: &CacheKey) -> Option<CachedPermissions> {
        let mut cache = self.cache.write().await;

        if let Some(cached) = cache.get(key) {
            if !cached.is_expired(self.ttl) {
                return Some(cached.clone());
            }
            // Expired - remove it
            cache.pop(key);
        }

        None
    }

    /// Set permissions in cache.
    pub async fn set(&self, key: CacheKey, permissions: CachedPermissions) {
        let mut cache = self.cache.write().await;
        cache.put(key, permissions);
    }

    /// Invalidate all entries for a session.
    ///
    /// Call this when a user logs out to ensure their cached permissions are removed.
    ///
    /// # Performance
    ///
    /// O(n) where n is the number of entries in the cache.
    pub async fn invalidate_session(&self, session_id: &str) {
        let mut cache = self.cache.write().await;

        let keys_to_remove: Vec<CacheKey> = cache
            .iter()
            .filter(|(key, _)| key.session_id == session_id)
            .map(|(key, _)| key.clone())
            .collect();

        for key in keys_to_remove {
            cache.pop(&key);
        }
    }

    /// Invalidate all entries for a workspace.
    ///
    /// Call this when workspace permissions change (e.g., role assignments, ACL updates).
    ///
    /// # Performance
    ///
    /// O(n) where n is the number of entries in the cache.
    pub async fn invalidate_workspace(&self, workspace_id: &str) {
        let mut cache = self.cache.write().await;

        let keys_to_remove: Vec<CacheKey> = cache
            .iter()
            .filter(|(key, _)| key.workspace_id == workspace_id)
            .map(|(key, _)| key.clone())
            .collect();

        for key in keys_to_remove {
            cache.pop(&key);
        }
    }

    /// Clear the entire cache.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Get the current number of entries in the cache.
    pub async fn len(&self) -> usize {
        let cache = self.cache.read().await;
        cache.len()
    }

    /// Check if the cache is empty.
    pub async fn is_empty(&self) -> bool {
        let cache = self.cache.read().await;
        cache.is_empty()
    }

    /// Get the configured TTL for this cache.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

impl Clone for PermissionCache {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
            ttl: self.ttl,
        }
    }
}
