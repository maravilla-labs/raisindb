//! MVCC-correct candidate resolution over a set of geohash cells.
//!
//! # The stale-entry bug this exists to fix
//!
//! `find_within_radius` and `scan_cells` each carried their own copy of the
//! iteration loop, and both got MVCC resolution wrong the same way:
//!
//! ```text
//! // Skip tombstones
//! if value.as_ref() == b"T" {
//!     continue;                            // <-- node NOT recorded as seen
//! }
//! ...
//! if distance <= radius_meters {
//!     seen_nodes.insert(node_id.clone());  // <-- only recorded on a LIVE match
//! }
//! ```
//!
//! Revisions sort **descending** within a cell prefix, so a node's newest entry is
//! reached first. When that newest entry was a tombstone the loop skipped it
//! *without* recording the node, then reached the node's older, still-live entry and
//! emitted it. Two observed consequences: a DELETED node kept matching
//! `ST_DWITHIN` forever, and a MOVED node matched at BOTH its old and its new
//! location. Same defect class as the reference index's false positives.
//!
//! # Why a patched `continue` would NOT have been enough
//!
//! The obvious fix — insert into `seen_nodes` on the tombstone branch too — is
//! correct *within one cell* and wrong *across cells*, which is the case that
//! actually matters. An update writes tombstones at the OLD geometry's cells and
//! live entries at the NEW geometry's cells, and a radius query routinely scans
//! both (they are usually neighbours). Whichever cell the iterator happened to
//! reach first would then decide the node's fate: old cell first and the node
//! disappears entirely — a false NEGATIVE, strictly worse than the bug being fixed.
//!
//! So resolution has to be **global across the scanned cells**, not per cell:
//!
//! 1. per cell, the newest in-window revision for each node;
//! 2. across cells, the newest of those per node;
//! 3. at equal revisions, a LIVE entry beats a tombstone.
//!
//! Step 3 is what makes an update and a delete both come out right, since an update
//! emits a tombstone and a live entry at the *same* revision (old cells and new
//! cells respectively) whereas a delete emits only tombstones. Prefer-live therefore
//! means "moved" resolves to the new location and "deleted" resolves to absent.

use super::{SpatialCandidate, SpatialIndexRepository};
use crate::indexing::SPATIAL_TOMBSTONE;
use crate::repositories::spatial_index::entry::SpatialEntry;
use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_storage::spatial::SpatialPreFilter;
use std::collections::HashMap;

/// Default hard ceiling on index entries visited inside ONE geohash cell prefix.
///
/// Overridable per storage instance via
/// `RocksDBConfig::spatial_max_entries_per_cell`.
///
/// # The unbounded thing this bounds
///
/// Resolution prefix-iterates each cell and VISITS EVERY KEY in it, including
/// every superseded revision and every tombstone. `answered_in_cell` stops a node
/// from being re-DECIDED but not from being re-ITERATED — the `continue` still
/// costs an iterator step.
///
/// For static places that is harmless: a place is written once and its cell holds
/// one entry per node. For a **tracked object** it is the dominant cost, and the
/// distribution is counter-intuitive. At a COARSE precision a vehicle circulating
/// an airport stays inside the SAME cell across every update, so that one prefix
/// accumulates roughly two entries per position update (the new live entry and the
/// tombstone of the old one, both in that cell) without bound. At a FINE precision
/// the vehicle moves between cells and entries spread thin. **Coarse cells are
/// where read cost concentrates.**
///
/// One vehicle at one update per second for 24h is ~86,400 updates and on the
/// order of 1.7e5 entries in its precision-6 prefix; 200 vehicles share ~3e7. A
/// query that was single-digit milliseconds on day one is seconds by day two.
/// RocksDB compaction does not help: the revision is IN the key
/// (`keys::spatial_keys`), so superseded entries are DISTINCT keys, not
/// overwritten versions, and nothing collapses them.
///
/// # What the cap does, and what it deliberately does not do
///
/// It bounds the *latency* of one cell scan and reports the truncation as a TYPED
/// error ([`Error::SpatialBudgetExceeded`]), which the SQL executor recognises and
/// re-plans against: the query degrades to the planner's spatial fallback — an
/// ordinary scan with the spatial predicate retained per row — instead of failing.
/// That preserves "a spatial query must never return FEWER rows than the truth"
/// AND "a spatial query must not fall over because one cell got fat". The budget
/// is a latency bound, never a correctness boundary. Pruning the accumulation
/// itself is the compaction filter's job (`crate::spatial::compaction`).
///
/// # A seek optimisation that looks obvious and does NOT work
///
/// One is tempted to seek past a node's remaining revisions once
/// `answered_in_cell` has it. It does not apply. The key is
/// `…geo\0{property}\0{cell}\0{revision:16}\0{node_id}`, so WITHIN a cell prefix
/// the ordering is by REVISION first and `node_id` only as a tiebreak: every
/// node's revisions are INTERLEAVED with every other node's. There is nothing
/// contiguous to seek past, and an implementer who assumes otherwise will write a
/// scan that silently skips live entries.
pub const DEFAULT_SPATIAL_MAX_ENTRIES_PER_CELL: usize = 250_000;

/// Whether a candidate's bounding box can intersect the query envelope.
fn bbox_intersects(candidate: &[f64; 4], query: &[f64; 4]) -> bool {
    candidate[0] <= query[2]
        && candidate[2] >= query[0]
        && candidate[1] <= query[3]
        && candidate[3] >= query[1]
}

/// The winning entry for one node across every scanned cell.
struct Resolved {
    revision: HLC,
    geohash: String,
    /// `None` when the winning revision is a tombstone.
    entry: Option<SpatialEntry>,
}

impl Resolved {
    /// Whether `candidate` supersedes `self`.
    ///
    /// Strictly newer always wins. At an equal revision a live entry beats a
    /// tombstone — see the module docs for why that is what makes moves and
    /// deletes both resolve correctly.
    fn superseded_by(&self, revision: HLC, is_live: bool) -> bool {
        revision > self.revision || (revision == self.revision && is_live && self.entry.is_none())
    }
}

impl SpatialIndexRepository {
    /// Resolve every live candidate in `cells` as of `max_revision`.
    ///
    /// # Guarantees
    ///
    /// - **Tombstone-correct** and **cross-cell consistent**: see the module docs.
    /// - **Deduplicated**: a node indexed at several precisions (normal — every
    ///   configured precision writes a cell) appears at most once.
    /// - **Pre-filtered before any node fetch**: bucket and bbox rejections happen
    ///   off the index value alone, so a floor-filtered proximity query never pays
    ///   for the non-matching floors' node records. That per-candidate
    ///   `nodes().get(...)` was the dominant cost of a selective spatial scan.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_live_candidates(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        cells: &[String],
        max_revision: &HLC,
        prefilter: &SpatialPreFilter,
    ) -> Result<Vec<SpatialCandidate>> {
        let cf = cf_handle(&self.db, cf::SPATIAL_INDEX)?;
        let mut winners: HashMap<String, Resolved> = HashMap::new();

        for cell in cells {
            let prefix = keys::spatial_index_geohash_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                cell,
            );

            // Within one cell the iterator yields revisions newest-first, so the
            // first in-window revision seen for a node is that cell's answer. Track
            // which nodes this cell has already answered for.
            let mut answered_in_cell: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut visited_in_cell = 0usize;

            for item in self.db.prefix_iterator_cf(cf, &prefix) {
                let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

                if !key.starts_with(&prefix) {
                    break;
                }

                visited_in_cell += 1;
                if visited_in_cell > self.max_entries_per_cell {
                    // Loud, and never silently short: the caller treats a truncated
                    // resolution as "the index cannot answer this completely".
                    tracing::warn!(
                        cell = %cell,
                        property = %property_name,
                        workspace = %workspace,
                        visited = visited_in_cell,
                        "Spatial cell prefix exceeded {} entries; this is superseded-revision \
                         accumulation, typical of a high-frequency position property indexed at a \
                         coarse precision. Reduce the precision set for this property, or model \
                         position history on separate nodes under a different property name.",
                        self.max_entries_per_cell
                    );
                    // A TYPED signal, not a string: the SQL executor recognises
                    // it and re-plans onto the spatial fallback, so the query
                    // degrades to a correct-but-slow row scan instead of
                    // failing. See `Error::is_spatial_budget_exceeded`.
                    return Err(Error::SpatialBudgetExceeded {
                        workspace: workspace.to_string(),
                        property: property_name.to_string(),
                        cell: cell.to_string(),
                        limit: self.max_entries_per_cell,
                    });
                }

                let Some(parsed) = keys::parse_spatial_index_key(&key) else {
                    tracing::warn!(
                        cell = %cell,
                        property = %property_name,
                        "Unparseable spatial index key; skipping"
                    );
                    continue;
                };

                // Revisions after the read snapshot are invisible. They are NOT
                // resolutions: an older revision may still be the visible one.
                if parsed.revision > *max_revision {
                    continue;
                }

                if !answered_in_cell.insert(parsed.node_id.to_string()) {
                    continue;
                }

                let is_tombstone = value.as_ref() == SPATIAL_TOMBSTONE;

                match winners.get(parsed.node_id) {
                    Some(existing) if !existing.superseded_by(parsed.revision, !is_tombstone) => {
                        continue;
                    }
                    _ => {}
                }

                let entry = if is_tombstone {
                    None
                } else {
                    match SpatialEntry::decode(&value) {
                        Some(e) => Some(e),
                        // An undecodable value is not evidence of absence; treat it
                        // as a resolution to "no usable entry" so an older entry
                        // cannot resurrect the node, and warn.
                        None => {
                            tracing::warn!(
                                node_id = %parsed.node_id,
                                property = %property_name,
                                "Undecodable spatial index value; treating node as unindexed"
                            );
                            None
                        }
                    }
                };

                winners.insert(
                    parsed.node_id.to_string(),
                    Resolved {
                        revision: parsed.revision,
                        geohash: parsed.geohash.to_string(),
                        entry,
                    },
                );
            }
        }

        let mut out = Vec::with_capacity(winners.len());
        for (node_id, resolved) in winners {
            let Some(entry) = resolved.entry else {
                continue; // tombstoned: deleted, or moved away from every scanned cell
            };
            if !entry.matches_bucket(prefilter.bucket_eq.as_deref()) {
                continue;
            }
            if let Some(query_bbox) = prefilter.bbox {
                // A legacy value's bbox collapsed to its centroid, so this stays
                // correct for point data and merely loses selectivity for extended
                // geometries written before the value record existed.
                if !bbox_intersects(&entry.bbox, &query_bbox) {
                    continue;
                }
            }
            out.push(SpatialCandidate {
                node_id,
                geohash: resolved.geohash,
                revision: resolved.revision,
                entry,
            });
        }

        Ok(out)
    }
}
