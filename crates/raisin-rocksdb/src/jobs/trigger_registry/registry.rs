//! Thread-safe trigger registry with cached lookups

use super::snapshot::TriggerRegistrySnapshot;
use super::types::CachedTrigger;
use arc_swap::ArcSwap;
use raisin_error::Result;
use raisin_storage::Storage;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Thread-safe trigger registry with cached lookups
///
/// Provides fast, lock-free reads with periodic background refreshes.
pub struct TriggerRegistry<S: Storage> {
    /// Current immutable snapshot (lock-free reads via arc-swap)
    pub(super) current: ArcSwap<TriggerRegistrySnapshot>,
    /// Storage backend for loading triggers
    pub(super) storage: Arc<S>,
    /// Mutex to prevent concurrent reloads
    reload_lock: Mutex<()>,
    /// Time-to-live before snapshot is considered stale
    ttl: Duration,
}

impl<S: Storage> TriggerRegistry<S> {
    /// Create a new trigger registry
    ///
    /// # Arguments
    ///
    /// * `storage` - Storage backend for querying triggers
    /// * `ttl` - Time-to-live for cached snapshots (default: 5 minutes)
    pub fn new(storage: Arc<S>, ttl: Duration) -> Self {
        Self {
            current: ArcSwap::new(Arc::new(TriggerRegistrySnapshot::empty())),
            storage,
            reload_lock: Mutex::new(()),
            ttl,
        }
    }

    /// Check if the current snapshot needs refresh
    pub fn needs_refresh(&self) -> bool {
        let snapshot = self.current.load();
        snapshot.loaded_at.elapsed() > self.ttl
    }

    /// Quick check: could this event possibly match any triggers?
    ///
    /// This is an O(1) operation that checks inverted indexes.
    /// Returns false only if there's definitely no possible match.
    /// Returns true if matches are possible (but not guaranteed).
    pub fn could_have_matches(&self, workspace: &str, node_type: &str) -> bool {
        let snapshot = self.current.load();
        snapshot.could_have_matches(workspace, node_type)
    }

    /// Get candidate triggers for an event
    ///
    /// Returns triggers that might match based on workspace, node_type, and event_kind.
    /// The caller must still perform detailed matching for:
    /// - Path glob patterns
    /// - Property filters
    pub fn get_candidates(
        &self,
        workspace: &str,
        node_type: &str,
        event_kind: &str,
    ) -> Vec<CachedTrigger> {
        let snapshot = self.current.load();
        snapshot.get_candidates(workspace, node_type, event_kind)
    }

    /// Invalidate and reload triggers from storage
    ///
    /// This method is thread-safe and uses try_lock to prevent concurrent reloads.
    /// If another thread is already reloading, this call returns immediately.
    pub async fn invalidate(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        // Try to acquire reload lock - if another thread is reloading, skip
        let lock = match self.reload_lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => {
                tracing::debug!("Trigger registry reload already in progress, skipping");
                return Ok(());
            }
        };

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            "Reloading trigger registry"
        );

        let start = Instant::now();
        let snapshot = self.load_snapshot(tenant_id, repo_id, branch).await?;
        let elapsed = start.elapsed();

        // Atomically swap in new snapshot
        let old_snapshot = self.current.swap(Arc::new(snapshot));

        tracing::info!(
            trigger_count = self.current.load().triggers.len(),
            old_version = old_snapshot.version,
            new_version = self.current.load().version,
            elapsed_ms = elapsed.as_millis(),
            "Trigger registry reloaded"
        );

        drop(lock);
        Ok(())
    }
}
