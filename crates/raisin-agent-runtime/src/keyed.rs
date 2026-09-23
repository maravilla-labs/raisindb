// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The per-run commit mutex: short, in-process, queueing.
//!
//! Every read → transition → commit of one run in this process runs under it,
//! for microseconds. Controls take ONLY this lock, never the execution lease,
//! so a stop never waits behind a model call. Entries retire when the last
//! holder releases, so the map tracks runs IN FLIGHT, not runs ever seen.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex, OwnedMutexGuard};

/// Per-key async mutexes.
pub struct KeyedMutex<K: Eq + Hash + Clone> {
    locks: StdMutex<HashMap<K, Arc<Mutex<()>>>>,
}

impl<K: Eq + Hash + Clone> Default for KeyedMutex<K> {
    fn default() -> Self {
        Self {
            locks: StdMutex::new(HashMap::new()),
        }
    }
}

/// Exclusive hold on one key.
pub struct KeyedGuard<'a, K: Eq + Hash + Clone> {
    owner: &'a KeyedMutex<K>,
    key: K,
    mutex: Arc<Mutex<()>>,
    guard: Option<OwnedMutexGuard<()>>,
}

impl<K: Eq + Hash + Clone> KeyedMutex<K> {
    /// Wait for and take `key`.
    pub async fn lock(&self, key: K) -> KeyedGuard<'_, K> {
        let mutex = self
            .locks
            .lock()
            .expect("keyed mutex poisoned")
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let guard = mutex.clone().lock_owned().await;
        KeyedGuard {
            owner: self,
            key,
            mutex,
            guard: Some(guard),
        }
    }

    /// Keys currently tracked.
    pub fn len(&self) -> usize {
        self.locks.lock().expect("keyed mutex poisoned").len()
    }

    /// Whether no key is tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: Eq + Hash + Clone> Drop for KeyedGuard<'_, K> {
    fn drop(&mut self) {
        self.guard.take();
        if let Ok(mut locks) = self.owner.locks.lock() {
            // The map's reference plus ours: nobody else is waiting.
            if Arc::strong_count(&self.mutex) <= 2 {
                locks.remove(&self.key);
            }
        }
    }
}
