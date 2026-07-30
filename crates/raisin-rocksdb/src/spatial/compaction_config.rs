//! Operator-facing configuration for the spatial index compaction filter.
//!
//! Split from [`super::compaction`], which owns the filter itself and the
//! correctness argument behind these knobs — read that module's docs before
//! changing a default.

/// Operator-facing configuration for the spatial index compaction filter.
#[derive(Debug, Clone)]
pub struct SpatialCompactionConfig {
    /// Whether superseded spatial index entries are pruned during compaction.
    ///
    /// `false` restores the pre-filter behaviour exactly (nothing is ever
    /// removed) — the escape hatch if the filter ever misbehaves.
    pub enabled: bool,

    /// Maximum entries retained per node per cell, newest first.
    ///
    /// `1` keeps only the newest. The default of 8 leaves a short history for
    /// near-HEAD historical reads while still bounding a hot prefix at a
    /// constant. Zero is treated as 1 — the newest entry is never droppable.
    pub keep_revisions: usize,

    /// Wall-clock horizon, in seconds, derived from the entry's own HLC
    /// timestamp. A superseded entry older than this is dropped even if it is
    /// inside the `keep_revisions` budget. `0` disables time-based retention
    /// (count budget only).
    pub retention_secs: u64,

    /// Whether a node's newest-in-run tombstone may itself be dropped during a
    /// FULL compaction once it has aged out of the retention window. This is
    /// what actually reclaims the space of deleted geometry.
    pub drop_tombstones: bool,

    /// Safety valve on filter memory: once this many distinct node ids have been
    /// tracked inside ONE cell prefix, the filter stops pruning that prefix and
    /// keeps everything. Bounds the per-compaction footprint of the seen-map on
    /// a pathologically wide cell.
    pub max_tracked_nodes_per_cell: usize,
}

impl Default for SpatialCompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_revisions: 8,
            retention_secs: 3_600,
            drop_tombstones: true,
            max_tracked_nodes_per_cell: 1_000_000,
        }
    }
}

impl SpatialCompactionConfig {
    /// Disabled: nothing is ever pruned.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Keep only the newest entry per node per cell — no retention at all.
    ///
    /// Maximum reclamation, and the configuration under which historical spatial
    /// reads are least accurate. Used by the tests that measure the effect.
    pub fn newest_only() -> Self {
        Self {
            keep_revisions: 1,
            retention_secs: 0,
            ..Self::default()
        }
    }

    /// Apply environment overrides, so an operator can turn the filter off (or
    /// widen its retention) on a running deployment without a config change.
    ///
    /// * `RAISIN_SPATIAL_COMPACTION_FILTER` — `0`/`off`/`false` disables it.
    /// * `RAISIN_SPATIAL_KEEP_REVISIONS` — count budget, in revisions.
    /// * `RAISIN_SPATIAL_RETENTION_SECS` — wall-clock horizon, in seconds.
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var("RAISIN_SPATIAL_COMPACTION_FILTER") {
            match v.trim().to_ascii_lowercase().as_str() {
                "0" | "off" | "false" | "no" => self.enabled = false,
                "1" | "on" | "true" | "yes" => self.enabled = true,
                _ => {}
            }
        }
        if let Ok(Ok(n)) = std::env::var("RAISIN_SPATIAL_KEEP_REVISIONS").map(|v| v.trim().parse())
        {
            self.keep_revisions = n;
        }
        if let Ok(Ok(n)) = std::env::var("RAISIN_SPATIAL_RETENTION_SECS").map(|v| v.trim().parse())
        {
            self.retention_secs = n;
        }
        self
    }

    /// The effective count budget (never zero: the newest is never droppable).
    pub(super) fn effective_keep(&self) -> usize {
        self.keep_revisions.max(1)
    }
}
