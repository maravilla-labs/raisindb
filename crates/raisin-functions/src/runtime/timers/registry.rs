// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Timer registry for managing setTimeout/setInterval timers
//!
//! This module tracks pending timers and allows cancellation via clearTimeout.
//! Timers are implemented using tokio's async sleep with cancellation support.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::oneshot;

/// State for a single timer
pub struct TimerState {
    /// Sender to cancel the timer (dropping it also cancels)
    cancel_tx: Option<oneshot::Sender<()>>,
}

/// Registry for managing timers
///
/// This registry allows creating and cancelling timers that are used
/// by the setTimeout/clearTimeout implementation.
pub struct TimerRegistry {
    /// Active timers indexed by ID
    timers: DashMap<String, TimerState>,
    /// Counter for generating unique timer IDs
    next_id: AtomicU64,
}

impl TimerRegistry {
    /// Create a new timer registry
    pub fn new() -> Self {
        Self {
            timers: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Generate a new unique timer ID
    pub fn generate_id(&self) -> String {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        format!("timer_{}", id)
    }

    /// Register a timer with cancellation support
    ///
    /// Returns a receiver that will be signaled if the timer is cancelled.
    /// The timer ID should have been generated with `generate_id()`.
    pub fn register(&self, timer_id: String) -> oneshot::Receiver<()> {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.timers.insert(
            timer_id,
            TimerState {
                cancel_tx: Some(cancel_tx),
            },
        );
        cancel_rx
    }

    /// Cancel a timer by ID
    ///
    /// Returns true if the timer was found and cancelled, false if not found.
    pub fn cancel(&self, timer_id: &str) -> bool {
        if let Some((_, mut state)) = self.timers.remove(timer_id) {
            // Send cancel signal (if receiver is still listening)
            if let Some(tx) = state.cancel_tx.take() {
                let _ = tx.send(());
            }
            true
        } else {
            false
        }
    }

    /// Remove a timer from the registry (called when timer completes)
    pub fn remove(&self, timer_id: &str) {
        self.timers.remove(timer_id);
    }

    /// Get the number of active timers (for debugging/metrics)
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }
}

impl Default for TimerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_generate_unique_ids() {
        let registry = TimerRegistry::new();

        let id1 = registry.generate_id();
        let id2 = registry.generate_id();
        let id3 = registry.generate_id();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert!(id1.starts_with("timer_"));
    }

    #[test]
    fn test_register_and_cancel() {
        let registry = TimerRegistry::new();

        let id = registry.generate_id();
        let _cancel_rx = registry.register(id.clone());

        assert_eq!(registry.len(), 1);
        assert!(registry.cancel(&id));
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_cancel_nonexistent() {
        let registry = TimerRegistry::new();
        assert!(!registry.cancel("nonexistent"));
    }

    #[tokio::test]
    async fn test_cancel_receiver_signals() {
        let registry = Arc::new(TimerRegistry::new());

        let id = registry.generate_id();
        let cancel_rx = registry.register(id.clone());

        // Spawn a task that cancels after a short delay
        let registry_clone = registry.clone();
        let id_clone = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            registry_clone.cancel(&id_clone);
        });

        // Wait for cancel signal
        let result = cancel_rx.await;
        assert!(result.is_ok());
    }
}
