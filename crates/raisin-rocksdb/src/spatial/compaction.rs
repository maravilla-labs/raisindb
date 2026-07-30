//! A stateful RocksDB compaction filter that drops superseded spatial index
//! entries from `cf::SPATIAL_INDEX`.
//!
//! # The problem
//!
//! The revision is part of the key
//! (`{tenant}\0{repo}\0{branch}\0{ws}\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}`,
//! see [`crate::keys::spatial_keys`]), so an update writes a NEW key rather than
//! overwriting an old one. RocksDB therefore has nothing to collapse on its own:
//! every superseded revision and every tombstone survives forever, and
//! `resolve_live_candidates` VISITS all of them. At a COARSE precision a tracked
//! object stays inside the same cell across every update, so that one prefix
//! accumulates ~2 entries per update without bound — one vehicle at 1 Hz for 24 h
//! puts ~1.7e5 entries in its precision-6 prefix. Query latency goes from
//! milliseconds to seconds within days and never recovers. A rebuild does not
//! help: it writes MORE tombstones. See `docs/OPEN-ITEMS.md` §2.99.
//!
//! # Why a *stateful* filter, and why it is correct
//!
//! `~rev` sits immediately after the geohash, so within one cell prefix keys are
//! presented to the filter in DESCENDING-revision order, with different nodes'
//! revisions interleaved (that interleaving is exactly why a "seek past this
//! node's revisions" scan optimisation does not work). Compaction presents keys
//! in sorted order, so all entries of one cell prefix arrive contiguously.
//!
//! The filter therefore tracks the current cell prefix and, per node inside it,
//! how many entries it has already kept. The FIRST entry seen for a node is that
//! node's newest revision *within this compaction* and is ALWAYS kept; later ones
//! are older and are kept only while they fall inside the retention window (see
//! below). The per-node state resets when the cell prefix changes.
//!
//! ## Partial visibility is safe in the right direction
//!
//! A compaction only sees the files it is compacting. If a node's newest entry
//! lives in a file that is not part of this run, the filter sees an OLDER entry
//! first and keeps it as if it were the newest. That is the safe direction: the
//! filter can only ever keep too much, never drop the live entry, because
//! "remove" is only ever decided after having *seen* a strictly newer entry for
//! the same node in the same cell during the same run. Pruning is consequently
//! INCREMENTAL and converges as levels merge. Do not "fix" this by consulting the
//! DB from inside the filter — a read from a compaction thread is a deadlock, and
//! the incremental behaviour is the whole reason the filter is safe.
//!
//! ## Tombstones
//!
//! A tombstone (value `b"T"`) shadows every older live entry for that node in
//! that cell. Dropping it would UNSHADOW an older live entry and resurrect
//! deleted geometry — the exact bug class the spatial pass has been eliminating.
//! It may only be dropped when nothing older can survive outside this run, which
//! is precisely `CompactionFilterContext::is_full_compaction`: the run includes
//! all data files for the range, so every older entry for that node in that cell
//! is also in this run and is purged by the same pass. When a node's newest entry
//! is a droppable tombstone the node is purged from that cell entirely.
//!
//! RocksDB's `level` argument is deliberately NOT used for this: "bottommost"
//! cannot be derived from a level number without also knowing `num_levels` and
//! whether this run's output is bottommost, whereas `is_full_compaction` is
//! reported directly and is exactly the guarantee needed.
//!
//! # MVCC time travel, and why there is a retention window
//!
//! Spatial time travel IS reachable. `resolve_live_candidates` takes a
//! `max_revision`; the SQL analyzer's `__revision = <n>` predicate
//! (`raisin-sql/src/analyzer/semantic/predicates.rs`) is stripped from the WHERE
//! clause and threaded into `ExecutionContext::max_revision`, which
//! `scan_executors/spatial_scan.rs` passes straight through. So
//! `SELECT ... WHERE __revision = 342 AND ST_DWithin(...)` is a genuine
//! historical spatial read on every SQL surface (HTTP, WS, pgwire). (The HTTP/WS
//! `rev/{revision}` REST APIs do NOT reach spatial — they go through
//! `NodeService::at_revision`, which does node/list reads only. Branch reads are
//! a different key prefix and each branch is read at its OWN head, so
//! cross-branch time travel is structurally impossible here.)
//!
//! Pruning unconditionally is therefore NOT provably safe, and this filter does
//! not do it. Retention is the explicit trade:
//!
//! * the newest entry per node per cell is **always** kept, so a query at branch
//!   HEAD — the overwhelming majority, and the default when no `__revision` is
//!   given — is always exactly as correct as before;
//! * a superseded entry is kept while it is within BOTH
//!   [`SpatialCompactionConfig::keep_revisions`] (a count budget) and
//!   [`SpatialCompactionConfig::retention_secs`] (a wall-clock horizon derived
//!   from the entry's own HLC timestamp).
//!
//! Both bounds are needed. The horizon alone does not bound a hot cell (1 Hz for
//! one hour is still 7,200 entries); the count alone lets a low-frequency
//! property retain useless decade-old revisions. Intersected, a hot prefix
//! reaches a steady state of `keep_revisions` entries per node and RECOVERS,
//! which is the property §2.99 is about.
//!
//! **Documented consequence:** a spatial query at a revision older than the
//! retention window resolves against whatever survived, so it may return a node
//! at a coarser point in its history or omit it. It never returns a *wrong-branch*
//! or *resurrected* row. Operators who need exact historical spatial reads raise
//! `retention_secs` / `keep_revisions`, or set `enabled = false`. A planner-side
//! gate that routed `__revision`-scoped spatial predicates to
//! `build_spatial_fallback_scan` (a row scan, always exact) would remove the
//! trade-off entirely; that lives in `raisin-sql-execution` and is filed as
//! follow-up work rather than done here.
//!
//! # Cost
//!
//! `filter` runs on RocksDB's compaction threads for every key in the CF. It uses
//! the zero-copy [`crate::keys::parse_spatial_index_key`] and allocates only when
//! the cell prefix changes or a node id is first seen in a prefix. A key that
//! fails to parse is always KEPT — never remove something you could not read.

use super::compaction_config::SpatialCompactionConfig;
use super::compaction_state::{CellPrefix, NodeState};
use crate::indexing::SPATIAL_TOMBSTONE;
use crate::keys::parse_spatial_index_key;
use raisin_hlc::HLC;
use rocksdb::compaction_filter::{CompactionFilter, Decision};
use rocksdb::compaction_filter_factory::{CompactionFilterContext, CompactionFilterFactory};
use std::collections::HashMap;
use std::ffi::{CStr, CString};

/// Counters for one compaction run, logged when the run ends.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpatialPruneStats {
    pub visited: u64,
    pub removed_superseded: u64,
    pub removed_tombstones: u64,
    pub unparseable: u64,
}

/// The stateful filter. One instance per compaction run (see
/// [`SpatialPruneFilterFactory`]); RocksDB guarantees a single thread drives it.
pub struct SpatialPruneFilter {
    config: SpatialCompactionConfig,
    /// True when this run includes ALL data files for its range — the only
    /// condition under which a tombstone may be dropped.
    is_full_compaction: bool,
    /// Reference time for the retention horizon, captured once per run so every
    /// key in the run is judged against the same instant.
    now_ms: u64,
    prefix: Option<CellPrefix>,
    seen: HashMap<String, NodeState>,
    /// Set when `max_tracked_nodes_per_cell` is exceeded for the current prefix.
    prefix_over_budget: bool,
    stats: SpatialPruneStats,
    name: CString,
}

impl SpatialPruneFilter {
    /// Construct a filter as if for a compaction run with the given visibility.
    ///
    /// `is_full_compaction` must be `true` only when the run includes every data
    /// file for its key range.
    pub fn new(config: SpatialCompactionConfig, is_full_compaction: bool) -> Self {
        Self::with_clock(config, is_full_compaction, HLC::now().timestamp_ms)
    }

    /// As [`Self::new`], with an explicit reference time. Tests use it so a
    /// retention window is deterministic rather than wall-clock dependent.
    pub fn with_clock(
        config: SpatialCompactionConfig,
        is_full_compaction: bool,
        now_ms: u64,
    ) -> Self {
        Self {
            config,
            is_full_compaction,
            now_ms,
            prefix: None,
            seen: HashMap::new(),
            prefix_over_budget: false,
            stats: SpatialPruneStats::default(),
            name: CString::new("raisin_spatial_prune_filter").expect("static name is valid"),
        }
    }

    /// Whether `revision` is inside the wall-clock retention horizon.
    ///
    /// A revision stamped in the future (clock skew across a cluster) is treated
    /// as in-window — keeping too much is the safe direction.
    fn within_retention(&self, revision: &HLC) -> bool {
        if self.config.retention_secs == 0 {
            return false;
        }
        let horizon_ms = self.config.retention_secs.saturating_mul(1_000);
        self.now_ms.saturating_sub(revision.timestamp_ms) <= horizon_ms
    }

    /// Counters accumulated so far. Exposed for tests and diagnostics.
    pub fn stats(&self) -> SpatialPruneStats {
        self.stats
    }

    /// The decision for one key/value, factored out of the trait method so tests
    /// can drive it directly with synthetic keys.
    pub fn decide(&mut self, key: &[u8], value: &[u8]) -> Decision {
        if !self.config.enabled {
            return Decision::Keep;
        }
        self.stats.visited += 1;

        // Never remove what could not be read.
        let Some(parsed) = parse_spatial_index_key(key) else {
            self.stats.unparseable += 1;
            return Decision::Keep;
        };

        if !self.prefix.as_ref().is_some_and(|p| p.matches(&parsed)) {
            let mut prefix = self.prefix.take().unwrap_or_default();
            prefix.adopt(&parsed);
            self.prefix = Some(prefix);
            self.seen.clear();
            self.prefix_over_budget = false;
        }

        if self.prefix_over_budget {
            return Decision::Keep;
        }

        match self.seen.get(parsed.node_id).copied() {
            Some(NodeState::PurgeRest) => {
                self.stats.removed_superseded += 1;
                Decision::Remove
            }
            Some(NodeState::Kept(kept)) => {
                if kept < self.config.effective_keep() && self.within_retention(&parsed.revision) {
                    self.seen
                        .insert(parsed.node_id.to_string(), NodeState::Kept(kept + 1));
                    Decision::Keep
                } else {
                    // Every remaining entry for this node is strictly older, so
                    // it violates the same bound. Short-circuit the rest.
                    self.seen
                        .insert(parsed.node_id.to_string(), NodeState::PurgeRest);
                    self.stats.removed_superseded += 1;
                    Decision::Remove
                }
            }
            None => {
                if self.seen.len() >= self.config.max_tracked_nodes_per_cell {
                    self.prefix_over_budget = true;
                    return Decision::Keep;
                }
                // First entry seen for this node in this prefix => its newest.
                let droppable_tombstone = value == SPATIAL_TOMBSTONE
                    && self.config.drop_tombstones
                    && self.is_full_compaction
                    && !self.within_retention(&parsed.revision);

                if droppable_tombstone {
                    self.seen
                        .insert(parsed.node_id.to_string(), NodeState::PurgeRest);
                    self.stats.removed_tombstones += 1;
                    Decision::Remove
                } else {
                    self.seen
                        .insert(parsed.node_id.to_string(), NodeState::Kept(1));
                    Decision::Keep
                }
            }
        }
    }
}

impl CompactionFilter for SpatialPruneFilter {
    fn filter(&mut self, _level: u32, key: &[u8], value: &[u8]) -> Decision {
        self.decide(key, value)
    }

    fn name(&self) -> &CStr {
        &self.name
    }
}

impl Drop for SpatialPruneFilter {
    fn drop(&mut self) {
        if self.stats.removed_superseded > 0 || self.stats.removed_tombstones > 0 {
            tracing::debug!(
                visited = self.stats.visited,
                removed_superseded = self.stats.removed_superseded,
                removed_tombstones = self.stats.removed_tombstones,
                unparseable = self.stats.unparseable,
                full_compaction = self.is_full_compaction,
                "Spatial index compaction pruned superseded entries"
            );
        }
    }
}

/// Creates one [`SpatialPruneFilter`] per compaction run.
///
/// A factory is required rather than a bare filter: the filter is STATEFUL
/// (per-cell, per-node state), so each run must start from a clean slate, and the
/// run's `is_full_compaction` flag — the tombstone-dropping precondition — is
/// only available here.
pub struct SpatialPruneFilterFactory {
    config: SpatialCompactionConfig,
    name: CString,
}

impl SpatialPruneFilterFactory {
    /// Wire this onto `cf::SPATIAL_INDEX` options and NO other column family:
    /// the filter parses keys in the spatial index's format and would keep
    /// (never remove) everything else, but the per-key parse cost is pure waste.
    pub fn new(config: SpatialCompactionConfig) -> Self {
        Self {
            config,
            name: CString::new("raisin_spatial_prune_filter_factory")
                .expect("static name is valid"),
        }
    }
}

impl CompactionFilterFactory for SpatialPruneFilterFactory {
    type Filter = SpatialPruneFilter;

    fn create(&mut self, context: CompactionFilterContext) -> Self::Filter {
        SpatialPruneFilter::new(self.config.clone(), context.is_full_compaction)
    }

    fn name(&self) -> &CStr {
        &self.name
    }
}
