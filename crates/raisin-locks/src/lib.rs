// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Atomic acquire / tie-breaker primitives for RaisinDB.
//!
//! This crate provides a backend-pluggable [`LockManager`] used to coordinate
//! "claim a finite thing without overselling" workloads (airline seats, ticket
//! sales, leader election, arbitrary critical sections).
//!
//! Two semantics are exposed on the same backend:
//!
//! * **Lease-lock** — [`LockManager::try_acquire`] takes a short, expiring lease
//!   on an arbitrary `key` and returns a [`LockGuard`] containing a monotonically
//!   increasing **fencing token**. A crashed/paused holder cannot deadlock the
//!   resource because the lease expires; a holder that was paused past its lease
//!   cannot corrupt state because its (now stale) fencing token is rejected by
//!   the guarded write. See the module docs for the fencing pattern.
//! * **Counting reservation** — [`LockManager::claim`] atomically reserves `n`
//!   units from a named pool, never letting the pool go below zero. This is the
//!   "N seats left" primitive.
//!
//! ## Backends
//!
//! * [`InProcessLockManager`] — zero-dependency, `DashMap`-backed. **Single node
//!   only.** Correct for single-process deployments; does *not* coordinate across
//!   a cluster.
//! * `RedisLockManager` (feature `redis`) — uses a linearizable Redis instance
//!   (`SET NX PX` + Lua) so multiple RaisinDB nodes share one source of truth.
//!
//! Pick one at runtime via [`LocksConfig`] / [`build`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use raisin_error::Result;
use serde::{Deserialize, Serialize};

mod inprocess;
pub use inprocess::InProcessLockManager;

#[cfg(feature = "redis")]
mod redis_backend;
#[cfg(feature = "redis")]
pub use redis_backend::RedisLockManager;

/// A monotonically increasing token handed out on every successful acquire.
///
/// Pass this into the write that the lock protects and have the resource reject
/// writes carrying a token lower than the last one it accepted — this closes the
/// "expired-lease holder resumes and clobbers a newer holder" race.
pub type FencingToken = u64;

/// Proof of a successfully held lease.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LockGuard {
    /// The acquired key.
    pub key: String,
    /// Fencing token for this acquisition (strictly increasing per manager).
    pub token: FencingToken,
    /// Wall-clock expiry of the lease, in unix milliseconds.
    pub expires_at_ms: u64,
}

/// Backend-agnostic atomic acquire / inventory primitive.
#[async_trait]
pub trait LockManager: Send + Sync {
    /// Try **once** to acquire `key` on behalf of `owner` for `ttl`.
    ///
    /// Returns `Ok(Some(guard))` on success or `Ok(None)` if the key is currently
    /// held by someone else (the tie-breaker outcome — caller should back off).
    async fn try_acquire(&self, key: &str, owner: &str, ttl: Duration)
        -> Result<Option<LockGuard>>;

    /// Release `key` iff `token` still matches the current holder. Idempotent:
    /// returns `Ok(false)` if the lock was already gone or held by someone else.
    async fn release(&self, key: &str, token: FencingToken) -> Result<bool>;

    /// Extend the lease on `key` iff `token` still matches. Returns `Ok(false)`
    /// if the lease was lost (expired or taken over).
    async fn renew(&self, key: &str, token: FencingToken, ttl: Duration) -> Result<bool>;

    /// Atomically claim `n` units from `pool`, seeding the pool to `capacity` the
    /// first time it is touched. Returns `Ok(Some(remaining))` on success or
    /// `Ok(None)` if fewer than `n` units remain (sold out).
    async fn claim(&self, pool: &str, n: u64, capacity: u64) -> Result<Option<u64>>;

    /// Return `n` previously-claimed units to `pool`. Returns the new remaining
    /// count (clamped at `capacity` is the caller's responsibility).
    async fn release_claim(&self, pool: &str, n: u64) -> Result<u64>;
}

/// Shared handle threaded into every transport / runtime surface.
pub type LockManagerHandle = Arc<dyn LockManager>;

/// Which backend to use.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LockBackend {
    /// In-process `DashMap` backend — single node only.
    #[default]
    InProcess,
    /// Redis-backed distributed backend (requires the `redis` feature).
    Redis,
}

/// Configuration for the locks subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LocksConfig {
    /// Master switch. When false, no manager is constructed and the surfaces
    /// report "locks subsystem disabled".
    #[serde(default)]
    pub enabled: bool,
    /// Backend selection.
    #[serde(default)]
    pub backend: LockBackend,
    /// How often the in-process backend sweeps expired entries.
    #[serde(default = "default_reaper_interval_secs")]
    pub reaper_interval_secs: u64,
    /// Redis connection settings (only read when `backend = "redis"`).
    #[serde(default)]
    pub redis: RedisConfig,
}

fn default_reaper_interval_secs() -> u64 {
    30
}

impl Default for LocksConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: LockBackend::default(),
            reaper_interval_secs: default_reaper_interval_secs(),
            redis: RedisConfig::default(),
        }
    }
}

/// Redis backend connection settings.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RedisConfig {
    /// Connection URL, e.g. `redis://127.0.0.1:6379/0`.
    #[serde(default = "default_redis_url")]
    pub url: String,
    /// Key prefix applied to every lock/pool key (keeps multiple RaisinDB
    /// clusters isolated on a shared Redis).
    #[serde(default = "default_redis_namespace")]
    pub namespace: String,
}

fn default_redis_url() -> String {
    "redis://127.0.0.1:6379/0".to_string()
}

fn default_redis_namespace() -> String {
    "raisin:locks".to_string()
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
            namespace: default_redis_namespace(),
        }
    }
}

/// Build a [`LockManagerHandle`] from config. Returns `Ok(None)` when the
/// subsystem is disabled.
pub async fn build(config: &LocksConfig) -> Result<Option<LockManagerHandle>> {
    if !config.enabled {
        return Ok(None);
    }
    match config.backend {
        LockBackend::InProcess => {
            let mgr = InProcessLockManager::with_reaper(Duration::from_secs(
                config.reaper_interval_secs.max(1),
            ));
            Ok(Some(Arc::new(mgr)))
        }
        LockBackend::Redis => {
            #[cfg(feature = "redis")]
            {
                let mgr = RedisLockManager::connect(&config.redis).await?;
                Ok(Some(Arc::new(mgr)))
            }
            #[cfg(not(feature = "redis"))]
            {
                Err(raisin_error::Error::invalid_state(
                    "locks.backend = \"redis\" but the server was built without the `redis` feature",
                ))
            }
        }
    }
}

/// Current wall-clock time in unix milliseconds.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
