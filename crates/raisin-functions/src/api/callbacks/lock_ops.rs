// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Lock / inventory operation callback type definitions

use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Callback for acquiring a lease-lock.
///
/// Args: `key`, `ttl_ms`, optional `owner`. Returns the lock guard
/// (`{ key, token, expires_at_ms }`) on success or `None` when the key is held.
pub type LockAcquireCallback = Arc<
    dyn Fn(
            String,         // key
            i64,            // ttl_ms
            Option<String>, // owner
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for releasing a lease-lock. Returns `true` if released.
pub type LockReleaseCallback = Arc<
    dyn Fn(
            String, // key
            i64,    // fencing token
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send>>
        + Send
        + Sync,
>;

/// Callback for renewing a lease-lock. Returns `true` if renewed.
pub type LockRenewCallback = Arc<
    dyn Fn(
            String, // key
            i64,    // fencing token
            i64,    // ttl_ms
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send>>
        + Send
        + Sync,
>;

/// Callback for claiming units from an inventory pool.
///
/// Args: `pool`, `n`, `capacity`. Returns `{ remaining }` on success or
/// `None` when the pool has fewer than `n` units left.
pub type InventoryClaimCallback = Arc<
    dyn Fn(
            String, // pool
            i64,    // n
            i64,    // capacity
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for returning units to an inventory pool. Returns new remaining count.
pub type InventoryReleaseCallback = Arc<
    dyn Fn(
            String, // pool
            i64,    // n
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<i64>> + Send>>
        + Send
        + Sync,
>;
