//! One sync run, as the console renders it.

use serde::{Deserialize, Serialize};

/// A single sync run, as the console renders it.
///
/// This exists because `BatchStats` used to be computed, logged at `info!`, and
/// dropped — and production runs at `RUST_LOG=warn`, so the one record of what a
/// sync actually did was discarded before anyone could read it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncRun {
    /// Unix epoch seconds.
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// `"delta" | "full" | "remap"`.
    pub mode: String,
    /// What caused this run: `"push" | "schedule" | "manual" | "unknown"`.
    ///
    /// Without this you cannot tell a mount nothing is scheduling from a mount
    /// whose provider has gone quiet — they look identical from the outside.
    pub trigger: String,
    /// `"running" | "ok" | "error" | "auth_required" | "misconfigured" | "skipped"`.
    pub outcome: String,
    #[serde(default)]
    pub written: u64,
    #[serde(default)]
    pub skipped: u64,
    #[serde(default)]
    pub deleted: u64,
    /// Reserved-metadata stamps the write drain applied to nodes that already
    /// existed. Kept OUT of [`Self::written`], which an operator reads as
    /// "items imported from the provider" — and which the batcher's
    /// wholesale-rejection guard reads as "this mount can write at all".
    #[serde(default)]
    pub stamped: u64,
    /// Local edits this run pushed to the provider.
    ///
    /// Distinct from [`Self::stamped`]: a stamp is the local metadata write that
    /// records a push, and the drain also stamps without pushing (a converged
    /// baseline). `pushed` is the number that answers "is writeback working".
    #[serde(default)]
    pub pushed: u64,
    /// Edits still queued because the drain ended early (budget or Stop).
    ///
    /// The one counter that separates a mount that is caught up from one that is
    /// falling behind. `outcome: "ok"` is reported either way — a drain that
    /// stops cleanly with work left is a success, not an error.
    #[serde(default)]
    pub writeback_pending: u64,
    #[serde(default)]
    pub failed: u64,
    /// Mount-owned nodes this run's reconcile left alone because they sit under
    /// a path the mount's own filters now EXCLUDE.
    ///
    /// An excluded subtree is out of scope, not condemned: the walk stops
    /// descending into it, so those nodes never land in `seen`, and without this
    /// carve-out the very next clean run would prune every one of them — an
    /// unbounded delete nobody asked for, triggered by adding one exclude
    /// pattern. Surfaced here and not only in a log line because the operator who
    /// changed the pattern is the one who needs to see the count.
    #[serde(default)]
    pub retained_excluded: u64,
    /// Items this run walked (the backfill chunk size, in practice).
    #[serde(default)]
    pub items_done: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Set when the run finished but its state write was refused — the failure
    /// mode that was previously invisible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_write_skipped: Option<bool>,
}

impl SyncRun {
    /// A run that has just started.
    pub fn started(now: i64, mode: &str, trigger: &str) -> Self {
        Self {
            started_at: now,
            mode: mode.to_string(),
            trigger: trigger.to_string(),
            outcome: "running".to_string(),
            ..Default::default()
        }
    }

    /// Stamp the terminal fields.
    pub fn finish(&mut self, now: i64, outcome: &str, error: Option<String>) {
        self.finished_at = Some(now);
        self.duration_ms = Some(((now - self.started_at).max(0) as u64) * 1000);
        self.outcome = outcome.to_string();
        self.error = error;
    }
}
