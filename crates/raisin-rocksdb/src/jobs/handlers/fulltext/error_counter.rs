//! Per-(tenant, repo, branch, kind) fulltext indexing error counter.
//!
//! Today, when `do_batch_index` or `do_index_node` errors inside
//! `spawn_blocking`, the worker logs the failure at error/debug level
//! and bubbles it up so the job system marks the job as Failed. There
//! is no aggregate signal — you'd only notice when a customer reports
//! that search returns empty hits.
//!
//! This counter is the v0.1.29 forward-looking observability hook.
//! Each failure increments a per-key, per-kind counter and records
//! the most recent error message; the admin console polls a JSON
//! endpoint (`GET /fulltext/errors`) and surfaces the counts next to
//! the existing "Rebuild" / "Reconcile" buttons so an operator can
//! act before a ticket comes in.
//!
//! # Why in-memory only?
//!
//! Counters reset on process restart. That's deliberate:
//!
//! - The point of the metric is "is the system unhealthy *now*?" —
//!   if the process restarted, the answer is "we don't know yet, but
//!   the next failure will repopulate."
//! - A persistent RocksDB CF would be the third new CF in v0.1.29
//!   (after `pending_batch_ops`). The cost-benefit isn't there for
//!   what is essentially observability state.
//! - Historical errors (e.g. v0.1.28 corruption) are surfaced through
//!   `verify_index`'s health_score, not through this counter.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;

/// Classification of a fulltext indexing failure. Kept narrow so the
/// admin UI can show one number per kind ("47 commit errors, 3 lock
/// errors") instead of a free-text bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FulltextErrorKind {
    /// Tantivy `commit()` returned an error or `do_batch_index` /
    /// `do_index_node` itself failed.
    Commit,
    /// The `spawn_blocking` task panicked or was cancelled before it
    /// could call into Tantivy.
    Spawn,
    /// Failed to load context (repository config, node fetch from
    /// RocksDB) before we got far enough to call Tantivy.
    NodeFetch,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FulltextErrorStats {
    pub commit_errors: u64,
    pub spawn_errors: u64,
    pub node_fetch_errors: u64,
    /// Most recent error message across all kinds. Truncated to 512
    /// bytes so a runaway `Display` impl can't blow up memory.
    pub last_error: Option<String>,
    /// Unix-epoch seconds when `last_error` was recorded.
    pub last_error_at: Option<u64>,
    /// Which kind of error was most recent — useful when triaging.
    pub last_error_kind: Option<FulltextErrorKind>,
}

impl FulltextErrorStats {
    pub fn total(&self) -> u64 {
        self.commit_errors + self.spawn_errors + self.node_fetch_errors
    }

    fn bump(&mut self, kind: FulltextErrorKind, message: &str) {
        match kind {
            FulltextErrorKind::Commit => self.commit_errors += 1,
            FulltextErrorKind::Spawn => self.spawn_errors += 1,
            FulltextErrorKind::NodeFetch => self.node_fetch_errors += 1,
        }
        let mut truncated = message.to_string();
        if truncated.len() > 512 {
            truncated.truncate(512);
            truncated.push_str("…");
        }
        self.last_error = Some(truncated);
        self.last_error_at = Some(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        );
        self.last_error_kind = Some(kind);
    }
}

/// Key identifying one Tantivy index. Identical shape to `IndexKey`
/// but with `Serialize` so it can be returned in the HTTP snapshot
/// payload without an extra conversion.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct ErrorCounterKey {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
}

impl ErrorCounterKey {
    pub fn new(tenant_id: &str, repo_id: &str, branch: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
        }
    }
}

/// Process-wide counter. Cloned (cheap, `Arc` inside) into the
/// FulltextJobHandler and re-exposed via `RocksDBStorage` so the HTTP
/// layer can read it without going through the handler.
#[derive(Clone, Default)]
pub struct FulltextErrorCounter {
    inner: Arc<RwLock<HashMap<ErrorCounterKey, FulltextErrorStats>>>,
}

impl FulltextErrorCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one failure. Always lock-acquires (write); calls happen
    /// only on the error path, never on the hot success path, so
    /// contention is a non-issue.
    pub async fn record(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        kind: FulltextErrorKind,
        message: &str,
    ) {
        let key = ErrorCounterKey::new(tenant_id, repo_id, branch);
        let mut map = self.inner.write().await;
        let stats = map.entry(key).or_default();
        stats.bump(kind, message);

        tracing::warn!(
            tenant_id,
            repo_id,
            branch,
            kind = ?kind,
            total_for_key = stats.total(),
            "Fulltext indexing failure recorded"
        );
    }

    /// Snapshot the stats for one key. Returns the default
    /// (all-zero) stats if nothing has been recorded — the admin UI
    /// treats `total() == 0` as "all clear", which is the same
    /// observable behaviour as "no entry exists".
    pub async fn snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> FulltextErrorStats {
        let key = ErrorCounterKey::new(tenant_id, repo_id, branch);
        self.inner
            .read()
            .await
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop the stats for one key. Called after a successful
    /// rebuild/reconcile so the admin UI reflects a green state.
    pub async fn clear(&self, tenant_id: &str, repo_id: &str, branch: &str) {
        let key = ErrorCounterKey::new(tenant_id, repo_id, branch);
        self.inner.write().await.remove(&key);
    }

    /// All known stats keyed by (tenant, repo, branch). Useful for a
    /// future tenant-overview page; the current admin UI only uses
    /// `snapshot`.
    #[allow(dead_code)]
    pub async fn snapshot_all(&self) -> Vec<(ErrorCounterKey, FulltextErrorStats)> {
        self.inner
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn record_increments_and_remembers_last_message() {
        let c = FulltextErrorCounter::new();
        c.record("t", "r", "main", FulltextErrorKind::Commit, "boom")
            .await;
        c.record("t", "r", "main", FulltextErrorKind::NodeFetch, "missing")
            .await;

        let s = c.snapshot("t", "r", "main").await;
        assert_eq!(s.commit_errors, 1);
        assert_eq!(s.node_fetch_errors, 1);
        assert_eq!(s.total(), 2);
        assert_eq!(s.last_error.as_deref(), Some("missing"));
        assert_eq!(s.last_error_kind, Some(FulltextErrorKind::NodeFetch));
    }

    #[tokio::test]
    async fn snapshot_unknown_key_returns_zero() {
        let c = FulltextErrorCounter::new();
        let s = c.snapshot("nope", "nope", "main").await;
        assert_eq!(s.total(), 0);
        assert!(s.last_error.is_none());
    }

    #[tokio::test]
    async fn keys_are_isolated() {
        let c = FulltextErrorCounter::new();
        c.record("a", "r", "main", FulltextErrorKind::Commit, "x")
            .await;
        c.record("b", "r", "main", FulltextErrorKind::Commit, "y")
            .await;

        assert_eq!(c.snapshot("a", "r", "main").await.commit_errors, 1);
        assert_eq!(c.snapshot("b", "r", "main").await.commit_errors, 1);
    }

    #[tokio::test]
    async fn clear_drops_only_one_key() {
        let c = FulltextErrorCounter::new();
        c.record("a", "r", "main", FulltextErrorKind::Commit, "x")
            .await;
        c.record("b", "r", "main", FulltextErrorKind::Commit, "y")
            .await;
        c.clear("a", "r", "main").await;

        assert_eq!(c.snapshot("a", "r", "main").await.total(), 0);
        assert_eq!(c.snapshot("b", "r", "main").await.total(), 1);
    }

    #[tokio::test]
    async fn long_messages_are_truncated() {
        let c = FulltextErrorCounter::new();
        let huge = "x".repeat(2000);
        c.record("t", "r", "main", FulltextErrorKind::Commit, &huge)
            .await;
        let s = c.snapshot("t", "r", "main").await;
        let len = s.last_error.as_ref().unwrap().len();
        assert!(len <= 520, "expected truncation, got len={}", len);
    }
}
