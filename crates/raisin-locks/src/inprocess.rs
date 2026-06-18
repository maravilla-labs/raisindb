// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! In-process, single-node [`LockManager`] backed by `DashMap`.
//!
//! Correctness comes from `DashMap`'s per-entry write locks: an acquire decision
//! (inspect current holder → decide → write) happens while holding the entry
//! lock, so concurrent acquires on the same key are serialized. **This does not
//! coordinate across processes** — use the Redis backend for multi-node setups.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use raisin_error::Result;

use crate::{now_ms, FencingToken, LockGuard, LockManager};

#[derive(Debug, Clone)]
struct LockEntry {
    #[allow(dead_code)]
    owner: String,
    token: FencingToken,
    expires_at_ms: u64,
}

#[derive(Debug, Default)]
struct Inner {
    locks: DashMap<String, LockEntry>,
    pools: DashMap<String, AtomicU64>,
    fence: AtomicU64,
}

impl Inner {
    fn next_token(&self) -> FencingToken {
        self.fence.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn sweep_expired(&self) {
        let now = now_ms();
        self.locks.retain(|_, e| e.expires_at_ms > now);
    }
}

/// Single-node in-process lock manager.
#[derive(Debug, Clone)]
pub struct InProcessLockManager {
    inner: Arc<Inner>,
}

impl Default for InProcessLockManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessLockManager {
    /// Create a manager with no background reaper (expiry is still applied
    /// lazily on every `try_acquire`).
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::default()),
        }
    }

    /// Create a manager that also runs a background task sweeping expired locks
    /// on `interval`. The task stops automatically when the manager is dropped.
    pub fn with_reaper(interval: Duration) -> Self {
        let mgr = Self::new();
        let weak: Weak<Inner> = Arc::downgrade(&mgr.inner);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                match weak.upgrade() {
                    Some(inner) => inner.sweep_expired(),
                    None => break,
                }
            }
        });
        mgr
    }
}

#[async_trait]
impl LockManager for InProcessLockManager {
    async fn try_acquire(
        &self,
        key: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<Option<LockGuard>> {
        let now = now_ms();
        let expires_at_ms = now.saturating_add(ttl.as_millis() as u64);

        match self.inner.locks.entry(key.to_string()) {
            Entry::Occupied(mut e) => {
                if e.get().expires_at_ms > now {
                    // Still held by a live lease — tie-breaker loss.
                    return Ok(None);
                }
                // Lease expired: take it over with a fresh fencing token.
                let token = self.inner.next_token();
                e.insert(LockEntry {
                    owner: owner.to_string(),
                    token,
                    expires_at_ms,
                });
                Ok(Some(LockGuard {
                    key: key.to_string(),
                    token,
                    expires_at_ms,
                }))
            }
            Entry::Vacant(e) => {
                let token = self.inner.next_token();
                e.insert(LockEntry {
                    owner: owner.to_string(),
                    token,
                    expires_at_ms,
                });
                Ok(Some(LockGuard {
                    key: key.to_string(),
                    token,
                    expires_at_ms,
                }))
            }
        }
    }

    async fn release(&self, key: &str, token: FencingToken) -> Result<bool> {
        match self.inner.locks.entry(key.to_string()) {
            Entry::Occupied(e) if e.get().token == token => {
                e.remove();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn renew(&self, key: &str, token: FencingToken, ttl: Duration) -> Result<bool> {
        let expires_at_ms = now_ms().saturating_add(ttl.as_millis() as u64);
        match self.inner.locks.entry(key.to_string()) {
            Entry::Occupied(mut e) if e.get().token == token => {
                e.get_mut().expires_at_ms = expires_at_ms;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn claim(&self, pool: &str, n: u64, capacity: u64) -> Result<Option<u64>> {
        // Seed the pool to `capacity` on first touch; subsequent calls ignore it.
        let counter = self
            .inner
            .pools
            .entry(pool.to_string())
            .or_insert_with(|| AtomicU64::new(capacity));

        let mut cur = counter.load(Ordering::SeqCst);
        loop {
            if cur < n {
                return Ok(None);
            }
            match counter.compare_exchange_weak(cur, cur - n, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Ok(Some(cur - n)),
                Err(actual) => cur = actual,
            }
        }
    }

    async fn release_claim(&self, pool: &str, n: u64) -> Result<u64> {
        let counter = self
            .inner
            .pools
            .entry(pool.to_string())
            .or_insert_with(|| AtomicU64::new(0));
        let prev = counter.fetch_add(n, Ordering::SeqCst);
        Ok(prev.saturating_add(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn only_one_acquires_under_contention() {
        let mgr = Arc::new(InProcessLockManager::new());
        let ttl = Duration::from_secs(60);
        let mut handles = Vec::new();
        for i in 0..50 {
            let m = mgr.clone();
            handles.push(tokio::spawn(async move {
                m.try_acquire("seat:14A", &format!("owner-{i}"), ttl)
                    .await
                    .unwrap()
                    .is_some()
            }));
        }
        let mut winners = 0;
        for h in handles {
            if h.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(winners, 1, "exactly one acquirer must win");
    }

    #[tokio::test]
    async fn expired_lease_can_be_retaken() {
        let mgr = InProcessLockManager::new();
        let g = mgr
            .try_acquire("k", "a", Duration::from_millis(20))
            .await
            .unwrap();
        assert!(g.is_some());
        // Held: second acquire fails.
        assert!(mgr
            .try_acquire("k", "b", Duration::from_secs(60))
            .await
            .unwrap()
            .is_none());
        tokio::time::sleep(Duration::from_millis(40)).await;
        // Expired: now retakeable.
        assert!(mgr
            .try_acquire("k", "b", Duration::from_secs(60))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn fencing_tokens_strictly_increase() {
        let mgr = InProcessLockManager::new();
        let g1 = mgr
            .try_acquire("k", "a", Duration::from_millis(1))
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let g2 = mgr
            .try_acquire("k", "b", Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        assert!(g2.token > g1.token, "fencing token must increase");
    }

    #[tokio::test]
    async fn release_requires_matching_token() {
        let mgr = InProcessLockManager::new();
        let g = mgr
            .try_acquire("k", "a", Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        assert!(!mgr.release("k", g.token + 999).await.unwrap());
        assert!(mgr.release("k", g.token).await.unwrap());
        // Idempotent: releasing again is a no-op.
        assert!(!mgr.release("k", g.token).await.unwrap());
    }

    #[tokio::test]
    async fn claim_never_oversells() {
        let mgr = Arc::new(InProcessLockManager::new());
        let mut handles = Vec::new();
        for _ in 0..200 {
            let m = mgr.clone();
            handles.push(tokio::spawn(async move {
                m.claim("flight", 1, 100).await.unwrap().is_some()
            }));
        }
        let mut sold = 0;
        for h in handles {
            if h.await.unwrap() {
                sold += 1;
            }
        }
        assert_eq!(sold, 100, "exactly capacity units may be claimed");
        assert!(mgr.claim("flight", 1, 100).await.unwrap().is_none());
    }
}
