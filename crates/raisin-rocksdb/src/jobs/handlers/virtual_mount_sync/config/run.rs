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
    #[serde(default)]
    pub failed: u64,
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
