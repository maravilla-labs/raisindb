//! Expanding-radius k-nearest-neighbour search.
//!
//! # What was wrong before
//!
//! `find_nearest` was dead code *and* incorrect. Three separate defects:
//!
//! 1. It computed `let cells = spatial::center_and_neighbors(&center_hash);` and
//!    **never used the variable**, delegating instead to `find_within_radius` with a
//!    guessed radius.
//! 2. `results` was only assigned inside the `if candidates.len() >= k || precision
//!    <= 4` branch, so a loop that fell through left `results` empty and discarded
//!    every candidate it had found — the last iteration always overwrote the
//!    earlier, better ones.
//! 3. The stopping rule was `candidates.len() >= k`. **"Found k candidates" and
//!    "found the k nearest" are different questions.** A node one ring further out
//!    can be nearer than one already collected, because a geohash cell is a
//!    rectangle and the query point sits somewhere inside it, not at its centre.
//!
//! No planner site ever constructed `PhysicalPlan::SpatialKnnScan`, which is why
//! nothing noticed.
//!
//! # The stopping rule used here
//!
//! Expand a search RADIUS geometrically, and at each step plan the scan with
//! [`crate::spatial::plan_radius_scan`], which returns a **proven-complete cover**
//! of that radius. Filter candidates to `distance <= radius`. If at least `k`
//! survive, those are the k nearest globally — because the cover guarantees that
//! *every* node within `radius` was seen, so nothing outside the returned set can be
//! nearer than the k-th.
//!
//! Expanding the radius rather than the ring count is what makes this correct AND
//! terminating. The naive alternative — start at the finest indexed precision and add
//! rings — starves: at precision 11 (0.15 m cells) even sixteen rings covers under
//! 5 m, so a workspace whose nearest neighbour is 10 m away returns nothing. The
//! radius formulation lets the planner pick the right precision at every step, which
//! is exactly the job it already does for `ST_DWITHIN`.

use super::{haversine_distance, SpatialIndexRepository};
use crate::spatial;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::spatial::{ProximityResult, SpatialPreFilter};

/// How far a KNN search expanded, for diagnostics and tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnnRingPlan {
    /// Precision the final scan used.
    pub precision: usize,
    /// Radius, in metres, the final scan covered.
    pub radius_meters: f64,
}

/// Growth factor per step. Four keeps the number of steps small (a 1 m start reaches
/// global scale in ~12 iterations) while not over-fetching wildly on the step that
/// succeeds.
const GROWTH: f64 = 4.0;

/// Where to start. Roughly a building-scale neighbourhood — small enough that a dense
/// dataset answers on the first probe with 9 cell seeks, large enough not to waste
/// several probes on sparse data.
const INITIAL_RADIUS_METERS: f64 = 25.0;

/// Half the Earth's circumference: beyond this every point on the planet is inside the
/// radius, so there is nothing further to find.
const MAX_RADIUS_METERS: f64 = 20_037_508.0;

/// Find the k nearest neighbours to a point.
#[allow(clippy::too_many_arguments)]
pub(super) fn find_nearest(
    repo: &SpatialIndexRepository,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    center_lon: f64,
    center_lat: f64,
    k: usize,
    max_revision: &HLC,
    precisions: &[usize],
    prefilter: &SpatialPreFilter,
) -> Result<Vec<ProximityResult>> {
    if k == 0 {
        return Ok(Vec::new());
    }

    let precisions = if precisions.is_empty() {
        spatial::INDEX_PRECISIONS
    } else {
        precisions
    };

    let mut radius = INITIAL_RADIUS_METERS;
    // Best-effort fallback: if no radius ever yields k results we return the widest
    // complete answer we managed, which is every node the index could reach.
    let mut best: Vec<ProximityResult> = Vec::new();

    loop {
        let plan = spatial::plan_radius_scan(center_lon, center_lat, radius, precisions);
        let cells = match &plan {
            spatial::SpatialScanPlan::Covering { cells, .. } => cells.clone(),
            // This radius is not coverable within the cell budget. Widening usually
            // fixes it (a coarser precision becomes eligible), so keep going rather
            // than giving up — but never return a partial answer for this radius.
            spatial::SpatialScanPlan::NotCovering => {
                if radius >= MAX_RADIUS_METERS {
                    return Ok(best);
                }
                radius *= GROWTH;
                continue;
            }
        };

        let candidates = repo.resolve_live_candidates(
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            &cells,
            max_revision,
            prefilter,
        )?;

        let mut scored: Vec<ProximityResult> = candidates
            .into_iter()
            .filter_map(|c| {
                let d = haversine_distance(center_lon, center_lat, c.entry.lon, c.entry.lat);
                // Only nodes provably inside `radius` count, because only for those
                // is the cover complete. A node found outside it might have nearer
                // unseen siblings in cells this plan did not scan.
                (d <= radius).then(|| c.into_proximity(d))
            })
            .collect();
        scored.sort_by(|a, b| a.distance_meters.total_cmp(&b.distance_meters));

        if scored.len() >= k {
            scored.truncate(k);
            return Ok(scored);
        }

        // Keep the widest complete answer so far, in case we run out of planet.
        if scored.len() > best.len() {
            best = scored;
        }

        if radius >= MAX_RADIUS_METERS {
            best.truncate(k);
            return Ok(best);
        }
        radius = (radius * GROWTH).min(MAX_RADIUS_METERS);
    }
}
