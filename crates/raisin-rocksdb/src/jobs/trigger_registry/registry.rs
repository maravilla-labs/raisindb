//! Thread-safe trigger registry with per-scope cached lookups
//!
//! The registry serves a multi-tenant server: trigger definitions live per
//! (tenant, repo, branch) scope, so the quick-reject cache is keyed by that
//! scope. A single global snapshot (the previous design) is incorrect here —
//! an invalidation from one repo would swap in that repo's triggers and
//! events from every other scope would be quick-rejected and silently
//! dropped (production incident: a lost agent-continuation event hung a
//! conversation forever).
//!
//! Performance: reads take one `RwLock` read lock + one `HashMap` lookup
//! (no storage I/O), so the hot event path stays O(1). Reloads are rare
//! (trigger definition changes) and only replace their own scope's entry.

use super::snapshot::TriggerRegistrySnapshot;
use raisin_error::Result;
use raisin_storage::Storage;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Cache key: (tenant_id, repo_id, branch)
pub(super) type ScopeKey = (String, String, String);

/// Default upper bound on cached scopes. Each snapshot is small (parsed
/// trigger definitions + inverted indexes), but a multi-tenant server can
/// host an unbounded number of scopes — evict beyond this bound.
const DEFAULT_MAX_SCOPES: usize = 512;

/// Storage-agnostic per-scope snapshot cache.
///
/// Semantics:
/// - A FRESH snapshot for a scope is authoritative for quick-reject.
/// - A missing or STALE (past TTL) snapshot fails OPEN: the caller must
///   enqueue evaluation and let the trigger matcher decide against storage.
pub(super) struct SnapshotCache {
    snapshots: RwLock<HashMap<ScopeKey, Arc<TriggerRegistrySnapshot>>>,
    ttl: Duration,
    max_scopes: usize,
}

impl SnapshotCache {
    pub(super) fn new(ttl: Duration, max_scopes: usize) -> Self {
        Self {
            snapshots: RwLock::new(HashMap::new()),
            ttl,
            max_scopes,
        }
    }

    /// Get the snapshot for a scope if present AND fresh.
    pub(super) fn get_fresh(&self, key: &ScopeKey) -> Option<Arc<TriggerRegistrySnapshot>> {
        let map = self.snapshots.read().unwrap_or_else(|e| e.into_inner());
        let snapshot = map.get(key)?;
        if snapshot.loaded_at.elapsed() > self.ttl {
            return None;
        }
        Some(snapshot.clone())
    }

    /// Current version for a scope (0 when never loaded).
    pub(super) fn version_for(&self, key: &ScopeKey) -> u64 {
        let map = self.snapshots.read().unwrap_or_else(|e| e.into_inner());
        map.get(key).map(|s| s.version).unwrap_or(0)
    }

    /// Insert/replace a scope's snapshot, evicting beyond the bound.
    ///
    /// Eviction prefers stale entries, then the oldest by load time. The
    /// entry being inserted is never evicted.
    pub(super) fn insert(&self, key: ScopeKey, snapshot: Arc<TriggerRegistrySnapshot>) {
        let mut map = self.snapshots.write().unwrap_or_else(|e| e.into_inner());

        if !map.contains_key(&key) && map.len() >= self.max_scopes {
            // Drop stale entries first.
            map.retain(|_, s| s.loaded_at.elapsed() <= self.ttl);
            // Still full: evict the oldest entries.
            while map.len() >= self.max_scopes {
                let oldest = map
                    .iter()
                    .min_by_key(|(_, s)| s.loaded_at)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => {
                        map.remove(&k);
                    }
                    None => break,
                }
            }
        }

        map.insert(key, snapshot);
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.snapshots
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    #[cfg(test)]
    pub(super) fn contains(&self, key: &ScopeKey) -> bool {
        self.snapshots
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(key)
    }
}

/// Thread-safe trigger registry with per-scope cached lookups.
pub struct TriggerRegistry<S: Storage> {
    /// Per-scope quick-reject snapshots
    pub(super) cache: SnapshotCache,
    /// Storage backend for loading triggers
    pub(super) storage: Arc<S>,
    /// Mutex to prevent reload stampedes (reloads are rare; a skipped
    /// reload only leaves ONE scope's previous snapshot serving, and
    /// stale scopes fail open)
    reload_lock: Mutex<()>,
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
            cache: SnapshotCache::new(ttl, DEFAULT_MAX_SCOPES),
            storage,
            reload_lock: Mutex::new(()),
        }
    }

    /// Scope-aware quick check: could this event possibly match any triggers?
    ///
    /// O(1) against the scope's own snapshot. Returns `false` only when the
    /// event's scope has a FRESH snapshot that definitely excludes the event
    /// shape. Unknown or stale scopes fail OPEN — the trigger matcher
    /// performs the authoritative check against storage.
    pub fn could_have_matches_for_scope(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
    ) -> bool {
        let key: ScopeKey = (
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
        );
        match self.cache.get_fresh(&key) {
            // Belt-and-braces: the snapshot also records its own scope, so a
            // snapshot stored under the wrong key can never reject foreign
            // events.
            Some(snapshot) => {
                snapshot.could_have_matches_scoped(tenant_id, repo_id, branch, workspace, node_type)
            }
            // Never loaded, or past TTL: fail open.
            None => true,
        }
    }

    /// Make sure this scope has a FRESH snapshot, loading one if needed.
    ///
    /// The quick-reject is only authoritative for a loaded scope, and nothing
    /// used to load a scope until its first trigger EDIT — so most scopes
    /// failed open forever and every write in the system paid a full trigger
    /// evaluation (two unbounded workspace scans in the matcher). This makes
    /// the load event-driven: one storage read per scope per TTL, amortized
    /// across every event in that scope. Stampede-protected by `invalidate`'s
    /// try_lock; a skipped or failed load keeps failing open, which is the
    /// pre-existing behaviour.
    pub async fn ensure_loaded(&self, tenant_id: &str, repo_id: &str, branch: &str) {
        let key: ScopeKey = (
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
        );
        if self.cache.get_fresh(&key).is_some() {
            return;
        }
        if let Err(e) = self.invalidate(tenant_id, repo_id, branch).await {
            tracing::warn!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                error = %e,
                "trigger registry load failed; quick-reject stays open for this scope"
            );
        }
    }

    /// Invalidate and reload triggers for ONE (tenant, repo, branch) scope.
    ///
    /// Other scopes' snapshots are untouched. Uses try_lock to prevent
    /// reload stampedes; when skipped, the scope's previous snapshot keeps
    /// serving until it goes stale, at which point reads fail open.
    pub async fn invalidate(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        let lock = match self.reload_lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    "Trigger registry reload already in progress, skipping"
                );
                return Ok(());
            }
        };

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            "Reloading trigger registry scope"
        );

        let key: ScopeKey = (
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
        );

        let start = Instant::now();
        let snapshot = self.load_snapshot(tenant_id, repo_id, branch).await?;
        let elapsed = start.elapsed();

        let trigger_count = snapshot.triggers.len();
        let new_version = snapshot.version;
        self.cache.insert(key, Arc::new(snapshot));

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            trigger_count = trigger_count,
            new_version = new_version,
            elapsed_ms = elapsed.as_millis(),
            "Trigger registry scope reloaded"
        );

        drop(lock);
        Ok(())
    }
}
