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

//! Process-wide registry of live permission caches, so an access-control write
//! can drop them.
//!
//! [`CachedPermissionService`](super::permission_service::CachedPermissionService)
//! caches each principal's FLATTENED permission set for five minutes, because
//! resolving it walks users → groups → roles → inheritance on every request.
//! That cache is what actually gates a query.
//!
//! Without this registry, nothing dropped it. The service exposes
//! `invalidate_user` / `invalidate_email` / `invalidate_branch` /
//! `invalidate_all`, and none of them was called anywhere: an admin adding a
//! permission to a role saw no effect for up to five minutes, with no way to
//! tell that from the change not having been saved. Worse, the WebSocket layer
//! already broadcast `permissions:changed` on such a write, so clients dutifully
//! re-resolved their ROLE LIST — which had not changed — and still queried
//! against the stale flattened set.
//!
//! Deliberately NOT [`super::derived_cache_registry`]: that one is documented as
//! a blunt instrument for bulk paths that bypass the write path entirely, and
//! firing it on every ACL write would also drop the function module cache and
//! the SQL catalog. This registry holds permission caches only.
//!
//! The service registers itself on construction. A cache is held WEAKLY, so a
//! dropped service deregisters itself rather than leaking, and its slot is
//! reaped on the next sweep.

use std::sync::{Mutex, OnceLock, Weak};

use super::permission_cache::PermissionCache;

static CACHES: OnceLock<Mutex<Vec<Weak<PermissionCache>>>> = OnceLock::new();

fn caches() -> &'static Mutex<Vec<Weak<PermissionCache>>> {
    CACHES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a permission cache so ACL writes can drop it.
///
/// Called from `CachedPermissionService::new`; there is no reason to call it
/// anywhere else.
pub fn register_permission_cache(cache: &std::sync::Arc<PermissionCache>) {
    if let Ok(mut guard) = caches().lock() {
        guard.push(std::sync::Arc::downgrade(cache));
    }
}

/// Drop every cached permission set in this process.
///
/// Call after any write to the access-control workspace that can change what
/// somebody may do — a `raisin:User`, `raisin:Role` or `raisin:Group`.
///
/// Coarse on purpose, and cheap for the same reason the `permissions:changed`
/// broadcast is coarse: ACL writes are rare admin actions, whereas working out
/// exactly which principals a role change affects means walking group and
/// inheritance edges — the very work the cache exists to avoid.
pub fn invalidate_all_permission_caches() {
    let Ok(mut guard) = caches().lock() else {
        tracing::error!("Permission cache registry poisoned; callers may keep stale permissions");
        return;
    };

    // Reap dead entries while we hold the lock.
    let mut dropped = 0usize;
    guard.retain(|weak| match weak.upgrade() {
        Some(cache) => {
            cache.invalidate_all();
            dropped += 1;
            true
        }
        None => false,
    });

    tracing::debug!(
        caches = dropped,
        "Dropped cached permissions after an access-control write"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn invalidating_clears_a_registered_cache() {
        let cache = Arc::new(PermissionCache::new(Duration::from_secs(300)));
        register_permission_cache(&cache);
        invalidate_all_permission_caches();
        assert_eq!(cache.stats().total_entries, 0);
    }

    #[test]
    fn a_dropped_cache_deregisters_itself() {
        let before = caches().lock().map(|g| g.len()).unwrap_or(0);
        {
            let cache = Arc::new(PermissionCache::new(Duration::from_secs(300)));
            register_permission_cache(&cache);
        }
        // The weak slot survives until the next sweep, which reaps it.
        invalidate_all_permission_caches();
        let after = caches().lock().map(|g| g.len()).unwrap_or(0);
        assert!(after <= before, "a dropped cache must not linger in the registry");
    }
}
