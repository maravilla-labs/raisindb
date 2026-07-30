//! Radius -> geohash cell planning with a proven-complete cover guarantee.
//!
//! This module replaces the old `cells_for_radius`, which picked a precision by
//! walking 12 -> 1 over `precision_radius_meters` **without regard to which
//! precisions were actually indexed**, then scanned a single fixed 3x3 Moore
//! neighbourhood. Because `spatial_index_geohash_prefix` appends a `\0`
//! separator, a prefix matches ONLY entries at exactly that precision — so any
//! radius that resolved to an unindexed precision returned ZERO ROWS, silently.
//! With the old `&[4, 5, 6, 7, 8]` index set that left a hard usable window of
//! roughly 4.8 m to 39 km; sub-5 m indoor queries were unusable.
//!
//! # The cover is computed GEOMETRICALLY, not from the radius table
//!
//! `precision_radius_meters` is a nominal, rounded figure, and geohash cells
//! alternate between roughly square and 2:1 rectangles as precision increases. A
//! 3x3 block at precision 10 is ~3.6 m wide but only ~1.8 m tall, so "the nominal
//! cell radius is 1.2 m, therefore a 3x3 block covers a 1 m circle" is false — a
//! point 1 m due north escapes the block. That is exactly the kind of
//! almost-right reasoning that produces a silent missing row, and a brute-force
//! coverage test in `spatial/tests.rs` caught it.
//!
//! So the planner reads each candidate precision's **actual** cell extent from
//! `geohash_bounds` at the query latitude, converts to metres, and derives the
//! number of rings from that.

use super::ops::{encode_point, geohash_bounds, ring_expand};
use raisin_models::nodes::properties::MAX_SCAN_CELLS;

/// The outcome of planning a radius query against a known index precision set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialScanPlan {
    /// The cell set is a **proven-complete cover** of the query circle.
    ///
    /// Every node whose indexed position lies within `radius` of the centre is
    /// guaranteed to appear in at least one of these cells. Only in this case may
    /// the planner drop the predicate from the residual filter.
    Covering {
        /// The precision every cell in `cells` is expressed at.
        precision: usize,
        /// Cells to scan, deduplicated, centre first. Never empty.
        cells: Vec<String>,
    },
    /// No complete cover could be produced within [`MAX_SCAN_CELLS`], or the
    /// centre coordinate was invalid.
    ///
    /// The caller MUST NOT use the index: fall back to a scan with the spatial
    /// predicate retained as a row-level filter. Returning partial results here
    /// would be the silent-wrong-answer bug this whole design exists to remove.
    NotCovering,
}

impl SpatialScanPlan {
    /// The cells to scan, or an empty slice when not covering.
    pub fn cells(&self) -> &[String] {
        match self {
            Self::Covering { cells, .. } => cells,
            Self::NotCovering => &[],
        }
    }

    /// Whether the index may be trusted as complete for this query.
    pub fn is_covering(&self) -> bool {
        matches!(self, Self::Covering { .. })
    }
}

/// Metres per degree of longitude and of latitude at `lat`, on the shared mean
/// sphere.
fn metres_per_degree(lat: f64) -> (f64, f64) {
    let per_deg = raisin_geometry::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI / 180.0;
    (per_deg * lat.to_radians().cos().abs().max(1e-12), per_deg)
}

/// The smaller of a cell's width and height at `precision`, in metres, measured at
/// the query point's own latitude.
///
/// This is the conservative per-ring advance: a query point can sit anywhere inside
/// the centre cell, so an `n`-ring block guarantees at least `n * extent` metres of
/// clearance in every direction.
fn min_cell_extent_meters(lon: f64, lat: f64, precision: usize) -> Option<f64> {
    let hash = encode_point(lon, lat, precision)?;
    let (min_lon, min_lat, max_lon, max_lat) = geohash_bounds(&hash)?;
    let (m_per_lon, m_per_lat) = metres_per_degree(lat);
    let width = (max_lon - min_lon) * m_per_lon;
    let height = (max_lat - min_lat) * m_per_lat;
    let smallest = width.min(height);
    (smallest > 0.0 && smallest.is_finite()).then_some(smallest)
}

/// Plan the cells to scan for a radius query against `precisions`.
///
/// `precisions` is the set the index was **actually built at** (never the
/// configured set — see the `indexed` vs `configured` split in `spatial_state`).
/// Order and duplicates do not matter; only membership.
///
/// # How the precision is chosen
///
/// For every candidate precision, the number of rings needed to guarantee the cover
/// is derived from that precision's real cell extent. The winner is the precision
/// needing the **fewest cells**, breaking ties towards the finest — which yields the
/// tightest covered area for a given number of RocksDB prefix seeks. Seeks dominate
/// the cost of a selective spatial scan, so minimising cell count rather than
/// covered area is the right objective: a 1 m query resolves to 9 cells at precision
/// 8 (a 38 m neighbourhood, cheaply over-fetched and then Haversine-filtered) rather
/// than 841 cells at precision 11.
///
/// # Why correctness is decoupled from configuration
///
/// Whatever precisions exist, the chosen plan covers the circle. Too coarse and we
/// over-fetch; too fine and we ring-expand; either way the exact post-filter removes
/// only false positives. A sub-metre query against an index built solely at
/// precision 8 returns the RIGHT rows. That is what makes a heterogeneous cluster
/// mid-reindex safe: every node returns the same result set at differing latency.
pub fn plan_radius_scan(
    center_lon: f64,
    center_lat: f64,
    radius_meters: f64,
    precisions: &[usize],
) -> SpatialScanPlan {
    if !radius_meters.is_finite() || radius_meters < 0.0 || precisions.is_empty() {
        return SpatialScanPlan::NotCovering;
    }
    if !center_lon.is_finite() || !center_lat.is_finite() {
        return SpatialScanPlan::NotCovering;
    }

    let mut best: Option<(usize, usize, usize)> = None; // (cell_count, rings, precision)

    for &precision in precisions {
        if !(1..=12).contains(&precision) {
            continue;
        }
        let Some(extent) = min_cell_extent_meters(center_lon, center_lat, precision) else {
            continue;
        };

        // A query point may sit on the centre cell's edge, so clearance is
        // `rings * extent`, not `(rings + 0.5) * extent`. At least one ring: a
        // point-in-cell query still has to look at the eight neighbours, since the
        // point can be arbitrarily close to a boundary.
        let rings = if radius_meters <= 0.0 {
            0
        } else {
            ((radius_meters / extent).ceil() as usize).max(1)
        };

        let span = 2usize.saturating_mul(rings).saturating_add(1);
        let cell_count = span.saturating_mul(span);
        if cell_count > MAX_SCAN_CELLS {
            continue;
        }

        let better = match best {
            None => true,
            Some((best_count, _, best_precision)) => {
                cell_count < best_count || (cell_count == best_count && precision > best_precision)
            }
        };
        if better {
            best = Some((cell_count, rings, precision));
        }
    }

    let Some((_, rings, precision)) = best else {
        return SpatialScanPlan::NotCovering;
    };

    let Some(center_hash) = encode_point(center_lon, center_lat, precision) else {
        return SpatialScanPlan::NotCovering;
    };
    let cells = ring_expand(&center_hash, rings);
    if cells.is_empty() || cells.len() > MAX_SCAN_CELLS {
        return SpatialScanPlan::NotCovering;
    }

    SpatialScanPlan::Covering { precision, cells }
}

/// Plan a radius scan against the default precision set.
///
/// Convenience for callers with no policy in hand. Prefer [`plan_radius_scan`] with
/// the index's actual precisions.
pub fn cells_for_radius(center_lon: f64, center_lat: f64, radius_meters: f64) -> Vec<String> {
    plan_radius_scan(
        center_lon,
        center_lat,
        radius_meters,
        super::INDEX_PRECISIONS,
    )
    .cells()
    .to_vec()
}
