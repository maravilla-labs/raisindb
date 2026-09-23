// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Cancellation tokens owned by the runtime, keyed by `(run, op)`.
//!
//! A stop can land before the driver registers its token (the driver commits
//! `OperationStarted` first). The registry then leaves a TOMBSTONE, and
//! `register` returns an already-cancelled token. Together with the driver's
//! state re-check after `register`, no ordering leaves an operation running
//! uncancelled.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use crate::ids::{OperationId, RunId};

enum Slot {
    Live(CancellationToken),
    Tombstone { at_ms: u64 },
}

/// Per-operation cancellation tokens.
#[derive(Default)]
pub struct CancellationRegistry {
    inner: Mutex<HashMap<(RunId, OperationId), Slot>>,
}

impl CancellationRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The token for `(run, op)`; already cancelled when a tombstone exists.
    pub fn register(&self, run: &RunId, op: &OperationId) -> CancellationToken {
        let mut map = self.inner.lock().expect("cancel registry poisoned");
        let key = (run.clone(), op.clone());
        match map.get(&key) {
            Some(Slot::Live(token)) => token.clone(),
            Some(Slot::Tombstone { .. }) => {
                let token = CancellationToken::new();
                token.cancel();
                map.insert(key, Slot::Live(token.clone()));
                token
            }
            None => {
                let token = CancellationToken::new();
                map.insert(key, Slot::Live(token.clone()));
                token
            }
        }
    }

    /// Cancel `(run, op)`; leaves a tombstone when nothing is registered yet.
    pub fn cancel(&self, run: &RunId, op: &OperationId, now_ms: u64) {
        let mut map = self.inner.lock().expect("cancel registry poisoned");
        match map.get(&(run.clone(), op.clone())) {
            Some(Slot::Live(token)) => token.cancel(),
            Some(Slot::Tombstone { .. }) => {}
            None => {
                map.insert((run.clone(), op.clone()), Slot::Tombstone { at_ms: now_ms });
            }
        }
    }

    /// Forget `(run, op)`.
    pub fn clear(&self, run: &RunId, op: &OperationId) {
        self.inner
            .lock()
            .expect("cancel registry poisoned")
            .remove(&(run.clone(), op.clone()));
    }

    /// Drop tombstones older than `older_than_ms`.
    pub fn gc(&self, older_than_ms: u64) {
        self.inner
            .lock()
            .expect("cancel registry poisoned")
            .retain(|_, slot| !matches!(slot, Slot::Tombstone { at_ms } if *at_ms < older_than_ms));
    }

    /// Number of slots (tests).
    pub fn len(&self) -> usize {
        self.inner.lock().expect("cancel registry poisoned").len()
    }

    /// Whether empty (tests).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_before_register_yields_cancelled_token() {
        let reg = CancellationRegistry::new();
        let (run, op) = (RunId::from("r"), OperationId::from("r/op/1"));
        reg.cancel(&run, &op, 1);
        assert!(reg.register(&run, &op).is_cancelled());
    }

    #[test]
    fn cancel_after_register_cancels_live_token() {
        let reg = CancellationRegistry::new();
        let (run, op) = (RunId::from("r"), OperationId::from("r/op/1"));
        let token = reg.register(&run, &op);
        assert!(!token.is_cancelled());
        reg.cancel(&run, &op, 1);
        assert!(token.is_cancelled());
        reg.clear(&run, &op);
        assert!(reg.is_empty());
    }

    #[test]
    fn gc_drops_old_tombstones() {
        let reg = CancellationRegistry::new();
        reg.cancel(&RunId::from("r"), &OperationId::from("r/op/1"), 5);
        reg.gc(10);
        assert!(reg.is_empty());
    }
}
