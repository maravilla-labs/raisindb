//! Turning a stored geometry into the set of geohash cells it is indexed under.
//!
//! Two modes, selected per property by [`SpatialCoverMode`]:
//!
//! - **Centroid** (default, matches historical behaviour): one cell per
//!   configured precision, at the geometry's centroid.
//! - **Extent**: the cells covering the geometry's bounding box, so a query
//!   inside a large polygon finds it even when the query point's cell does not
//!   contain the centroid. Capped at [`MAX_COVER_CELLS`] per precision, degrading
//!   to centroid-plus-bbox-corners when the cap bites.
//!
//! Correctness note: `Centroid` is not a *correctness* compromise, because
//! bounding-box pushdown is `Envelope` mode — a superset filter that leaves the
//! original predicate in the residual filter. `Extent` only improves selectivity.

use super::normalize::normalize_geometry_for_index;
use super::ops::{encode_point, is_valid_coordinate};
use raisin_error::Result;
use raisin_models::nodes::properties::{
    GeoJson, Position, SpatialCoverMode, SpatialPolicy, MAX_COVER_CELLS,
};

/// The write-side description of one indexed geometry.
///
/// Produced once per geometry per revision and consumed by both the index writer
/// and the tombstone writer, which is what guarantees a superseded entry is
/// tombstoned at exactly the cells it was written at.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryCells {
    /// Cells to write, across every configured precision. Deduplicated.
    pub cells: Vec<String>,
    /// WGS84 centroid (lon, lat) — the value record's `lon`/`lat`.
    pub centroid: (f64, f64),
    /// WGS84 bounding box `[min_lon, min_lat, max_lon, max_lat]`.
    pub bbox: [f64; 4],
    /// Altitude extent, when the geometry carries any Z ordinate.
    pub z_range: Option<(f64, f64)>,
}

/// Compute the cells and metadata for indexing `geometry` under `policy`.
///
/// # SRID normalisation happens HERE, and only here
///
/// This is the single funnel every spatial write reaches — the transaction
/// context, the repository CRUD path, the replication apply path, the
/// delete/tombstone path and the reindex job all arrive through
/// `crate::indexing::spatial`, which calls nothing else to derive cells. So
/// normalising here is what makes "no write path can forget" a structural
/// property rather than a convention. See [`normalize_geometry_for_index`] for
/// why only the built-in projection tier is consulted.
///
/// The returned `centroid` and `bbox` are consequently always WGS84, matching
/// what [`GeometryCells`] documents, whatever CRS the geometry is stored in.
///
/// # Errors
///
/// The geometry's SRID is outside the built-in projection tier, so no index cell
/// can be derived for it. The caller must fail the write rather than store a
/// geometry no spatial query can ever find.
///
/// `Ok(None)` is the separate, benign case: the geometry has no usable position
/// (empty geometry, or coordinates outside the WGS84 domain). Such a geometry is
/// simply not indexed, which is correct because no query point can be near it.
pub fn cells_for_geometry(
    geometry: &GeoJson,
    policy: &SpatialPolicy,
) -> Result<Option<GeometryCells>> {
    let normalized = normalize_geometry_for_index(geometry)?;
    Ok(cells_for_wgs84_geometry(normalized.as_ref(), policy))
}

/// The cell derivation itself, on a geometry already known to be WGS84.
///
/// Split out so [`cells_for_geometry`] reads as "normalise, then cover" and so
/// the covering maths stays testable without a CRS in the picture.
fn cells_for_wgs84_geometry(geometry: &GeoJson, policy: &SpatialPolicy) -> Option<GeometryCells> {
    let centroid = geometry_centroid(geometry)?;
    let bbox = geometry_bbox(geometry)?;

    let mut cells: Vec<String> = Vec::with_capacity(policy.precisions.len());
    let mut seen = std::collections::HashSet::new();

    for &precision in &policy.precisions {
        let at_precision = match policy.cover {
            SpatialCoverMode::Centroid => encode_point(centroid.0, centroid.1, precision)
                .into_iter()
                .collect(),
            SpatialCoverMode::Extent => bbox_cells(&bbox, centroid, precision),
        };
        for cell in at_precision {
            if seen.insert(cell.clone()) {
                cells.push(cell);
            }
        }
    }

    if cells.is_empty() {
        return None;
    }

    Some(GeometryCells {
        cells,
        centroid,
        bbox,
        z_range: geometry.z_range(),
    })
}

/// Cells covering a bounding box at one precision, capped at
/// [`MAX_COVER_CELLS`].
///
/// The cap is mandatory: a country-sized polygon under `Extent` at precision 11
/// would otherwise want billions of cells and blow the write budget by many
/// orders of magnitude. When the projected count exceeds the cap we degrade to
/// the centroid cell plus the four bbox-corner cells — still strictly better
/// than centroid-only, and bounded.
fn bbox_cells(bbox: &[f64; 4], centroid: (f64, f64), precision: usize) -> Vec<String> {
    let [min_lon, min_lat, max_lon, max_lat] = *bbox;

    // Cell dimensions at this precision, read from the centre cell's own extent
    // rather than a table, so it stays exact for the geohash implementation.
    let centre = match encode_point(centroid.0, centroid.1, precision) {
        Some(h) => h,
        None => return Vec::new(),
    };
    let (lon_step, lat_step) = match super::ops::geohash_bounds(&centre) {
        Some((lo_lon, lo_lat, hi_lon, hi_lat)) => (hi_lon - lo_lon, hi_lat - lo_lat),
        None => return vec![centre],
    };
    if lon_step <= 0.0 || lat_step <= 0.0 {
        return vec![centre];
    }

    let cols = ((max_lon - min_lon) / lon_step).ceil() as usize + 1;
    let rows = ((max_lat - min_lat) / lat_step).ceil() as usize + 1;

    if cols.saturating_mul(rows) > MAX_COVER_CELLS {
        // Degrade: centroid + the four corners.
        let mut out = vec![centre];
        for (lon, lat) in [
            (min_lon, min_lat),
            (min_lon, max_lat),
            (max_lon, min_lat),
            (max_lon, max_lat),
        ] {
            if let Some(h) = encode_point(lon, lat, precision) {
                if !out.contains(&h) {
                    out.push(h);
                }
            }
        }
        return out;
    }

    let mut out = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let lon = min_lon + (c as f64 + 0.5) * lon_step;
            let lat = min_lat + (r as f64 + 0.5) * lat_step;
            // Clamp into the box so a half-step overshoot cannot leave it.
            let lon = lon.min(max_lon).max(min_lon);
            let lat = lat.min(max_lat).max(min_lat);
            if let Some(h) = encode_point(lon, lat, precision) {
                if !out.contains(&h) {
                    out.push(h);
                }
            }
        }
    }
    if out.is_empty() {
        out.push(centre);
    }
    out
}

/// The WGS84 bounding box of a geometry, as `[min_lon, min_lat, max_lon, max_lat]`.
///
/// Stored in the index value so a query can reject a candidate on its extent
/// before paying for a node fetch.
pub fn geometry_bbox(geometry: &GeoJson) -> Option<[f64; 4]> {
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    let mut any = false;

    geometry.for_each_position(&mut |p: &Position| {
        if !is_valid_coordinate(p.x, p.y) {
            return;
        }
        any = true;
        min_lon = min_lon.min(p.x);
        min_lat = min_lat.min(p.y);
        max_lon = max_lon.max(p.x);
        max_lat = max_lat.max(p.y);
    });

    if !any {
        return None;
    }
    Some([min_lon, min_lat, max_lon, max_lat])
}

/// Extract centroid coordinates from a GeoJSON geometry.
///
/// Delegates to `GeoJson::centroid`, which (since the Area A model work) covers
/// every geometry type including `Multi*` and `GeometryCollection` — the old
/// hand-rolled table here returned `None` for all of those, so a `MultiPolygon`
/// was silently unindexed.
pub fn geometry_centroid(geojson: &GeoJson) -> Option<(f64, f64)> {
    geojson
        .centroid()
        .map(|p| (p.x, p.y))
        .filter(|&(lon, lat)| {
            let valid = is_valid_coordinate(lon, lat);
            if !valid {
                tracing::warn!(
                    lon,
                    lat,
                    "Computed geometry centroid has invalid coordinates"
                );
            }
            valid
        })
}
