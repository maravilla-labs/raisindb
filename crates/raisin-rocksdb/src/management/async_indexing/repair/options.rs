//! How a repair runs ([`RepairOptions`]) and what it did ([`RepairReport`]).

use super::{BatchReport, CommitHook};
use super::{NodePathCounts, OrderedChildrenCounts, PropertyIndexCounts};
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

/// How to run a repair.
#[derive(Debug, Clone)]
pub struct RepairOptions {
    /// Report what would be written; write nothing.
    pub dry_run: bool,
    /// Commit when a batch reaches this many bytes.
    pub batch_bytes: usize,
    /// Refuse to start without 2x the CF size free on the data volume.
    pub check_headroom: bool,
    /// Stop as if crashed after this many committed batches (tests).
    pub stop_after_batches: Option<usize>,
    /// Average write rate cap between batches, in bytes per second; 0 means
    /// unlimited. Keeps a repair from saturating the disk a live node serves
    /// from.
    pub max_bytes_per_sec: u64,
    /// TEST hook run at the start of every commit (see [`CommitHook`]).
    pub before_commit: Option<CommitHook>,
    /// `property_index_verify` checks one live node in this many.
    pub sample_every: u64,
    /// TEST hook: the free bytes the disk precheck assumes instead of asking
    /// the volume.
    pub free_bytes_override: Option<u64>,
    /// `collapse_runs`: the column families to collapse (`None`: all of
    /// `history_gc::collapse::COLLAPSE_CFS`).
    pub collapse_cfs: Option<Vec<String>>,
    /// `collapse_runs`: an upper bound on the watermark — it can only be
    /// lowered, never raised past HEAD or `collapse_min_age`.
    pub collapse_watermark_cap: Option<HLC>,
    /// `collapse_runs`: nothing younger than this (wall clock) is collapsed,
    /// so transactions that allocated a revision before HEAD moved have
    /// committed. Default ten minutes, retention GC's floor.
    pub collapse_min_age: std::time::Duration,
    /// This node replicates. `run_repair` sets it from the configuration;
    /// collapse refuses in cluster mode (no causal-stability watermark).
    pub cluster_mode: bool,
    /// `collapse_runs`: a slice ends after scanning this many keys, even with
    /// nothing to delete, so the exclusive `(branch, CF)` hold, the iterator's
    /// snapshot and the uncommitted cursor stay bounded on redundancy-free data.
    pub collapse_slice_keys: u64,
    /// `collapse_runs`: a slice also ends after this long.
    pub collapse_slice_time: std::time::Duration,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            batch_bytes: 8 * 1024 * 1024,
            check_headroom: true,
            stop_after_batches: None,
            max_bytes_per_sec: 32 * 1024 * 1024,
            before_commit: None,
            sample_every: 16,
            free_bytes_override: None,
            collapse_cfs: None,
            collapse_watermark_cap: None,
            collapse_min_age: std::time::Duration::from_secs(600),
            cluster_mode: false,
            collapse_slice_keys: 100_000,
            collapse_slice_time: std::time::Duration::from_millis(250),
        }
    }
}

/// What one repair did on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairReport {
    pub branch: String,
    pub repair: String,
    pub dry_run: bool,
    /// The run picked up from a persisted cursor.
    pub resumed: bool,
    /// The run reached the end (false: stopped early, resumable).
    pub completed: bool,
    /// Writes queued/committed, batches and bytes.
    pub writes: BatchReport,
    /// PATH_INDEX entries scanned (path repair).
    pub scanned: u64,
    /// ORDERED_CHILDREN repair counts.
    pub ordered: OrderedChildrenCounts,
    /// NODE_PATH backfill counts.
    #[serde(default)]
    pub node_path: NodePathCounts,
    /// PROPERTY_INDEX rebuild / verify counts.
    #[serde(default)]
    pub property_index: PropertyIndexCounts,
    /// Run-collapse GC counts.
    #[serde(default)]
    pub collapse: crate::management::history_gc::collapse::CollapseCounts,
    /// Translation resync counts.
    #[serde(default)]
    pub translations: super::TranslationResyncCounts,
    /// Localized name index build counts.
    #[serde(default)]
    pub localized_names: crate::localized_name::rebuild::LocalizedNameCounts,
    /// Block-overlay delete tombstone counts (plan Phase 11c).
    #[serde(default)]
    pub block_overlays: super::BlockOverlayCounts,
}
