// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Production callback factories for lock / inventory operations.
//!
//! Each factory captures the shared [`LockManagerHandle`] and returns a callback
//! that delegates to it. Keys are namespaced by tenant/repo/branch so locks in
//! one scope never collide with another.

use std::sync::Arc;
use std::time::Duration;

use raisin_locks::{LockManager, LockManagerHandle};
use serde_json::Value;

use crate::api::{
    InventoryClaimCallback, InventoryReleaseCallback, LockAcquireCallback, LockReleaseCallback,
    LockRenewCallback,
};

/// Build the `{tenant}\0{repo}\0{branch}\0{name}` namespace prefix used to keep
/// locks isolated per scope (mirrors the storage key convention).
fn scoped(tenant_id: &str, repo_id: &str, branch: &str, name: &str) -> String {
    format!("{tenant_id}\0{repo_id}\0{branch}\0{name}")
}

pub fn create_lock_acquire(
    manager: LockManagerHandle,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> LockAcquireCallback {
    Arc::new(move |key: String, ttl_ms: i64, owner: Option<String>| {
        let manager = manager.clone();
        let scoped_key = scoped(&tenant_id, &repo_id, &branch, &key);
        let owner = owner.unwrap_or_else(|| "anonymous".to_string());
        Box::pin(async move {
            let ttl = Duration::from_millis(ttl_ms.max(0) as u64);
            let guard = manager.try_acquire(&scoped_key, &owner, ttl).await?;
            // Return the guard with the caller-facing (unscoped) key.
            Ok(guard.map(|g| {
                serde_json::json!({
                    "key": key,
                    "token": g.token,
                    "expires_at_ms": g.expires_at_ms,
                })
            }))
        })
    })
}

pub fn create_lock_release(
    manager: LockManagerHandle,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> LockReleaseCallback {
    Arc::new(move |key: String, token: i64| {
        let manager = manager.clone();
        let scoped_key = scoped(&tenant_id, &repo_id, &branch, &key);
        Box::pin(async move { manager.release(&scoped_key, token as u64).await })
    })
}

pub fn create_lock_renew(
    manager: LockManagerHandle,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> LockRenewCallback {
    Arc::new(move |key: String, token: i64, ttl_ms: i64| {
        let manager = manager.clone();
        let scoped_key = scoped(&tenant_id, &repo_id, &branch, &key);
        Box::pin(async move {
            let ttl = Duration::from_millis(ttl_ms.max(0) as u64);
            manager.renew(&scoped_key, token as u64, ttl).await
        })
    })
}

pub fn create_inventory_claim(
    manager: LockManagerHandle,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> InventoryClaimCallback {
    Arc::new(move |pool: String, n: i64, capacity: i64| {
        let manager = manager.clone();
        let scoped_pool = scoped(&tenant_id, &repo_id, &branch, &pool);
        Box::pin(async move {
            let remaining = manager
                .claim(&scoped_pool, n.max(0) as u64, capacity.max(0) as u64)
                .await?;
            Ok(remaining.map(|r| -> Value { serde_json::json!({ "remaining": r }) }))
        })
    })
}

pub fn create_inventory_release(
    manager: LockManagerHandle,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> InventoryReleaseCallback {
    Arc::new(move |pool: String, n: i64| {
        let manager = manager.clone();
        let scoped_pool = scoped(&tenant_id, &repo_id, &branch, &pool);
        Box::pin(async move {
            let remaining = manager.release_claim(&scoped_pool, n.max(0) as u64).await?;
            Ok(remaining as i64)
        })
    })
}
