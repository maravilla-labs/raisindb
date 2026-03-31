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

//! Cached permission service wrapping PermissionService with in-memory caching.

use std::sync::Arc;
use std::time::Duration;

use raisin_error::Result;
use raisin_models::permissions::ResolvedPermissions;
use raisin_storage::Storage;

use super::PermissionService;
use crate::services::permission_cache::{PermissionCache, SharedPermissionCache};

/// Cached permission service that wraps PermissionService with in-memory caching.
///
/// This provides TTL-based caching of resolved permissions to avoid repeated
/// database queries for the same user.
///
/// # Usage
///
/// ```rust,ignore
/// let service = CachedPermissionService::new(storage.clone(), Duration::from_secs(300));
///
/// // First call computes and caches
/// let perms = service.resolve_for_user_id(tenant, repo, branch, "user123").await?;
///
/// // Subsequent calls use cache until TTL expires
/// let perms = service.resolve_for_user_id(tenant, repo, branch, "user123").await?;
///
/// // Invalidate when user/role/group changes
/// service.invalidate_user("user123");
/// ```
pub struct CachedPermissionService<S: Storage> {
    inner: PermissionService<S>,
    cache: SharedPermissionCache,
}

impl<S: Storage> CachedPermissionService<S> {
    /// Create a new cached permission service with specified TTL.
    pub fn new(storage: Arc<S>, ttl: Duration) -> Self {
        Self {
            inner: PermissionService::new(storage),
            cache: Arc::new(PermissionCache::new(ttl)),
        }
    }

    /// Create a new cached permission service with default 5-minute TTL.
    pub fn with_default_ttl(storage: Arc<S>) -> Self {
        Self::new(storage, Duration::from_secs(300))
    }

    /// Create with a shared cache (for use across multiple service instances).
    pub fn with_shared_cache(storage: Arc<S>, cache: SharedPermissionCache) -> Self {
        Self {
            inner: PermissionService::new(storage),
            cache,
        }
    }

    /// Get the shared cache for use elsewhere.
    pub fn cache(&self) -> &SharedPermissionCache {
        &self.cache
    }

    /// Resolve permissions for a user by their email (cached).
    pub async fn resolve_for_email(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        email: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let cache_key = format!("{}:{}:{}:email:{}", tenant_id, repo_id, branch, email);

        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(Some(cached));
        }

        let result = self
            .inner
            .resolve_for_email(tenant_id, repo_id, branch, email)
            .await?;

        if let Some(ref perms) = result {
            self.cache.put(&cache_key, perms.clone());
            let user_key = format!(
                "{}:{}:{}:user:{}",
                tenant_id, repo_id, branch, perms.user_id
            );
            self.cache.put(&user_key, perms.clone());
        }

        Ok(result)
    }

    /// Resolve permissions for a user by their node ID (cached).
    pub async fn resolve_for_user_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        user_id: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let cache_key = format!("{}:{}:{}:user:{}", tenant_id, repo_id, branch, user_id);

        let result = self
            .cache
            .get_or_compute(&cache_key, || async {
                self.inner
                    .resolve_for_user_id(tenant_id, repo_id, branch, user_id)
                    .await
                    .map(|opt| {
                        opt.unwrap_or_else(|| ResolvedPermissions {
                            user_id: user_id.to_string(),
                            email: None,
                            direct_roles: vec![],
                            group_roles: vec![],
                            effective_roles: vec![],
                            groups: vec![],
                            permissions: vec![],
                            is_system_admin: false,
                            resolved_at: Some(std::time::Instant::now()),
                        })
                    })
            })
            .await?;

        if result.effective_roles.is_empty()
            && result.permissions.is_empty()
            && !result.is_system_admin
        {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }

    /// Resolve permissions for a user by their identity ID (cached).
    ///
    /// This looks up by the `user_id` property (identity from global auth),
    /// not by node UUID.
    pub async fn resolve_for_identity_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        identity_id: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let cache_key = format!(
            "{}:{}:{}:identity:{}",
            tenant_id, repo_id, branch, identity_id
        );

        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(Some(cached));
        }

        let result = self
            .inner
            .resolve_for_identity_id(tenant_id, repo_id, branch, identity_id)
            .await?;

        if let Some(ref perms) = result {
            self.cache.put(&cache_key, perms.clone());
            let user_key = format!(
                "{}:{}:{}:user:{}",
                tenant_id, repo_id, branch, perms.user_id
            );
            self.cache.put(&user_key, perms.clone());
        }

        Ok(result)
    }

    /// Resolve permissions for the anonymous role (cached, DEPRECATED).
    pub async fn resolve_anonymous(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<ResolvedPermissions> {
        let cache_key = format!("{}:{}:{}:anonymous", tenant_id, repo_id, branch);

        self.cache
            .get_or_compute(&cache_key, || async {
                self.inner
                    .resolve_anonymous(tenant_id, repo_id, branch)
                    .await
            })
            .await
    }

    /// Resolve permissions for the physical anonymous user (cached).
    ///
    /// This is the preferred method for anonymous access. It uses the physical
    /// anonymous user node for proper role-based permission inheritance.
    pub async fn resolve_anonymous_user(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let cache_key = format!("{}:{}:{}:anonymous_user", tenant_id, repo_id, branch);

        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(Some(cached));
        }

        let result = self
            .inner
            .resolve_anonymous_user(tenant_id, repo_id, branch)
            .await?;

        if let Some(ref perms) = result {
            self.cache.put(&cache_key, perms.clone());
        }

        Ok(result)
    }

    /// Invalidate cached permissions for a specific user.
    ///
    /// Call this when:
    /// - User's roles or groups change
    /// - User is deleted
    pub fn invalidate_user(&self, tenant_id: &str, repo_id: &str, branch: &str, user_id: &str) {
        let key = format!("{}:{}:{}:user:{}", tenant_id, repo_id, branch, user_id);
        self.cache.invalidate(&key);
    }

    /// Invalidate cached permissions for a user by email.
    pub fn invalidate_email(&self, tenant_id: &str, repo_id: &str, branch: &str, email: &str) {
        let key = format!("{}:{}:{}:email:{}", tenant_id, repo_id, branch, email);
        self.cache.invalidate(&key);
    }

    /// Invalidate all cached permissions for a branch.
    ///
    /// Call this when:
    /// - A role's permissions change
    /// - A group's roles change
    /// - Major permission restructuring occurs
    pub fn invalidate_branch(&self, tenant_id: &str, repo_id: &str, branch: &str) {
        let _prefix = format!("{}:{}:{}:", tenant_id, repo_id, branch);
        self.cache.invalidate_all();
        tracing::info!(
            tenant_id = tenant_id,
            repo_id = repo_id,
            branch = branch,
            "Invalidated all cached permissions for branch"
        );
    }

    /// Invalidate all cached permissions.
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> crate::services::permission_cache::CacheStats {
        self.cache.stats()
    }

    /// Access the underlying uncached service.
    pub fn inner(&self) -> &PermissionService<S> {
        &self.inner
    }
}
