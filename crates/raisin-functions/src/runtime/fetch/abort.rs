// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AbortController/AbortSignal registry for fetch cancellation
//!
//! This module manages abort controllers that allow JavaScript code to cancel
//! in-flight fetch requests. Each controller has a unique ID that links the
//! JavaScript AbortController to the Rust-side cancellation mechanism.

use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// State for a single abort controller
struct AbortControllerState {
    /// Whether the controller has been aborted
    aborted: AtomicBool,
    /// The abort reason (if any)
    reason: std::sync::Mutex<Option<String>>,
    /// Broadcast sender for notifying waiters
    sender: broadcast::Sender<()>,
}

impl AbortControllerState {
    fn new() -> Self {
        let (sender, _) = broadcast::channel(1);
        Self {
            aborted: AtomicBool::new(false),
            reason: std::sync::Mutex::new(None),
            sender,
        }
    }
}

/// Registry for managing abort controllers
///
/// This registry allows creating, querying, and triggering abort controllers
/// that are used by the W3C Fetch API implementation.
pub struct AbortRegistry {
    controllers: DashMap<String, Arc<AbortControllerState>>,
}

impl AbortRegistry {
    /// Create a new abort registry
    pub fn new() -> Self {
        Self {
            controllers: DashMap::new(),
        }
    }

    /// Create a new abort controller and return its ID
    pub fn create_controller(&self) -> String {
        let id = nanoid::nanoid!();
        let state = Arc::new(AbortControllerState::new());
        self.controllers.insert(id.clone(), state);
        id
    }

    /// Abort a controller by ID
    ///
    /// Returns true if the controller was found and aborted, false if not found.
    pub fn abort(&self, id: &str, reason: Option<String>) -> bool {
        if let Some(state) = self.controllers.get(id) {
            // Set aborted flag
            state.aborted.store(true, Ordering::SeqCst);

            // Store reason
            if let Ok(mut guard) = state.reason.lock() {
                *guard = reason;
            }

            // Notify any waiters (ignore send errors - no receivers is fine)
            let _ = state.sender.send(());

            true
        } else {
            false
        }
    }

    /// Check if a controller has been aborted
    pub fn is_aborted(&self, id: &str) -> bool {
        self.controllers
            .get(id)
            .map(|state| state.aborted.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Get the abort reason for a controller
    pub fn get_reason(&self, id: &str) -> Option<String> {
        self.controllers
            .get(id)
            .and_then(|state| state.reason.lock().ok().and_then(|guard| guard.clone()))
    }

    /// Wait for a controller to be aborted
    ///
    /// Returns immediately if already aborted, otherwise waits until aborted.
    /// Returns None if the controller doesn't exist.
    pub async fn wait_for_abort(&self, id: &str) -> Option<()> {
        let state = self.controllers.get(id)?;

        // Check if already aborted
        if state.aborted.load(Ordering::SeqCst) {
            return Some(());
        }

        // Subscribe to abort notifications
        let mut receiver = state.sender.subscribe();

        // Double-check after subscribing (race condition)
        if state.aborted.load(Ordering::SeqCst) {
            return Some(());
        }

        // Wait for abort signal
        let _ = receiver.recv().await;
        Some(())
    }

    /// Remove a controller from the registry
    ///
    /// Should be called when the fetch request completes to clean up resources.
    pub fn remove(&self, id: &str) {
        self.controllers.remove(id);
    }

    /// Get the number of active controllers (for debugging/metrics)
    pub fn len(&self) -> usize {
        self.controllers.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.controllers.is_empty()
    }
}

impl Default for AbortRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_abort() {
        let registry = AbortRegistry::new();

        let id = registry.create_controller();
        assert!(!registry.is_aborted(&id));

        registry.abort(&id, Some("user cancelled".to_string()));
        assert!(registry.is_aborted(&id));
        assert_eq!(registry.get_reason(&id), Some("user cancelled".to_string()));
    }

    #[test]
    fn test_nonexistent_controller() {
        let registry = AbortRegistry::new();

        assert!(!registry.is_aborted("nonexistent"));
        assert!(!registry.abort("nonexistent", None));
        assert!(registry.get_reason("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_wait_for_abort() {
        let registry = Arc::new(AbortRegistry::new());
        let id = registry.create_controller();

        // Spawn a task that aborts after a short delay
        let registry_clone = registry.clone();
        let id_clone = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            registry_clone.abort(&id_clone, None);
        });

        // Wait for abort
        let result = registry.wait_for_abort(&id).await;
        assert!(result.is_some());
        assert!(registry.is_aborted(&id));
    }

    #[tokio::test]
    async fn test_wait_for_abort_already_aborted() {
        let registry = AbortRegistry::new();
        let id = registry.create_controller();

        // Abort before waiting
        registry.abort(&id, None);

        // Should return immediately
        let result = registry.wait_for_abort(&id).await;
        assert!(result.is_some());
    }
}
