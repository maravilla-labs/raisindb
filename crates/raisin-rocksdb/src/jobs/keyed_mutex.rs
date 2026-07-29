// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! A map of mutexes keyed by an arbitrary value.
//!
//! Serializes work that must not run concurrently *for the same key* while
//! leaving different keys fully parallel — the shape both the index rebuild
//! locks and the flow-instance execution lock need.
//!
//! Waiters QUEUE rather than fail, which is the difference between this and
//! `raisin_locks`' try-acquire semantics: a second delivery for the same key
//! should proceed after the first, not be rejected.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex, OwnedMutexGuard};

/// Per-key mutexes, created on first use.
///
/// Entries are retired when the last waiter releases them, so a long-lived
/// process that touches many keys (one per flow instance, say) does not grow
/// a permanent entry per key.
pub struct KeyedMutex<K: Eq + Hash + Clone + Send + Sync + 'static> {
    /// A std mutex on purpose: the map is only ever touched for a
    /// get-or-insert and a remove, never across an await, so eviction can
    /// happen synchronously in `Drop` instead of spawning a task.
    locks: StdMutex<HashMap<K, Arc<Mutex<()>>>>,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> KeyedMutex<K> {
    /// Create an empty registry
    pub fn new() -> Self {
        Self {
            locks: StdMutex::new(HashMap::new()),
        }
    }

    /// Get the mutex for `key`, creating it if absent.
    ///
    /// The caller locks the returned mutex; holding the `Arc` keeps the entry
    /// alive, so it cannot be evicted out from under a waiter.
    pub fn mutex_for(&self, key: &K) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .expect("keyed mutex registry poisoned")
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Acquire `key` exclusively, waiting for any current holder.
    ///
    /// The returned guard also drops the registry entry when it is the last
    /// reference, keeping the map proportional to keys IN FLIGHT rather than
    /// to keys ever seen.
    pub async fn lock(self: &Arc<Self>, key: K) -> KeyedMutexGuard<K> {
        let mutex = self.mutex_for(&key);
        let guard = mutex.clone().lock_owned().await;
        KeyedMutexGuard {
            registry: self.clone(),
            key,
            mutex,
            guard: Some(guard),
        }
    }

    /// Number of keys currently tracked (monitoring and tests)
    pub fn len(&self) -> usize {
        self.locks
            .lock()
            .expect("keyed mutex registry poisoned")
            .len()
    }

    /// Whether any key is currently tracked
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> Default for KeyedMutex<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// Exclusive hold on one key. Releases, and retires the entry, on drop.
pub struct KeyedMutexGuard<K: Eq + Hash + Clone + Send + Sync + 'static> {
    registry: Arc<KeyedMutex<K>>,
    key: K,
    mutex: Arc<Mutex<()>>,
    /// Dropped before the entry is retired
    guard: Option<OwnedMutexGuard<()>>,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> Drop for KeyedMutexGuard<K> {
    fn drop(&mut self) {
        // Release first, so a waiter can take the mutex immediately.
        self.guard.take();

        // Then retire the entry if nobody else refers to it, so the map stays
        // proportional to keys IN FLIGHT. Two references means the map's and
        // ours; more means another waiter is queued and the entry must stay.
        let mut locks = match self.registry.locks.lock() {
            Ok(locks) => locks,
            Err(_) => return,
        };
        if Arc::strong_count(&self.mutex) <= 2 {
            locks.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_same_key_serializes() {
        let registry: Arc<KeyedMutex<String>> = Arc::new(KeyedMutex::new());

        let held = registry.lock("a".to_string()).await;

        let contender = {
            let registry = registry.clone();
            tokio::spawn(async move { registry.lock("a".to_string()).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!contender.is_finished(), "same key must wait");

        drop(held);
        tokio::time::timeout(Duration::from_secs(2), contender)
            .await
            .expect("waiter proceeds once released")
            .expect("task joins");
    }

    #[tokio::test]
    async fn test_different_keys_are_parallel() {
        let registry: Arc<KeyedMutex<String>> = Arc::new(KeyedMutex::new());
        let _a = registry.lock("a".to_string()).await;
        tokio::time::timeout(Duration::from_millis(200), registry.lock("b".to_string()))
            .await
            .expect("a different key must not block");
    }

    #[tokio::test]
    async fn test_entries_are_retired_after_release() {
        let registry: Arc<KeyedMutex<String>> = Arc::new(KeyedMutex::new());
        for i in 0..25 {
            drop(registry.lock(format!("key-{}", i)).await);
        }
        assert!(
            registry.is_empty(),
            "keys no longer in flight must not accumulate, found {}",
            registry.len()
        );
    }
}
