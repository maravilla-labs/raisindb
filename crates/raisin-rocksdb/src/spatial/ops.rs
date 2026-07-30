//! Core geohash encoding, decoding, and neighbor operations.

use super::INDEX_PRECISIONS;

/// Check whether the given longitude and latitude are finite and within the
/// valid WGS-84 range (-180..=180 lon, -90..=90 lat).
pub(super) fn is_valid_coordinate(lon: f64, lat: f64) -> bool {
    lon.is_finite()
        && lat.is_finite()
        && (-180.0..=180.0).contains(&lon)
        && (-90.0..=90.0).contains(&lat)
}

/// Encode a point (lon, lat) to a geohash string at the specified precision
pub fn encode_point(lon: f64, lat: f64, precision: usize) -> Option<String> {
    if !is_valid_coordinate(lon, lat) {
        return None;
    }
    geohash::encode(geohash::Coord { x: lon, y: lat }, precision)
        .map_err(|e| {
            tracing::warn!(lon, lat, precision, error = %e, "geohash encode failed");
            e
        })
        .ok()
}

/// Decode a geohash to its center point (lon, lat)
pub fn decode_geohash(hash: &str) -> Option<(f64, f64)> {
    geohash::decode(hash)
        .ok()
        .map(|(coord, _, _)| (coord.x, coord.y))
}

/// Get the bounding box of a geohash cell
///
/// Returns (min_lon, min_lat, max_lon, max_lat)
pub fn geohash_bounds(hash: &str) -> Option<(f64, f64, f64, f64)> {
    geohash::decode(hash).ok().map(|(coord, lon_err, lat_err)| {
        (
            coord.x - lon_err,
            coord.y - lat_err,
            coord.x + lon_err,
            coord.y + lat_err,
        )
    })
}

/// Get the 8 neighboring geohash cells (Moore neighborhood)
pub fn neighbors(hash: &str) -> Vec<String> {
    if hash.is_empty() {
        return Vec::new();
    }
    geohash::neighbors(hash)
        .map(|n| vec![n.n, n.ne, n.e, n.se, n.s, n.sw, n.w, n.nw])
        .unwrap_or_else(|e| {
            tracing::warn!(hash, error = %e, "geohash neighbors lookup failed");
            Vec::new()
        })
}

/// Get the center geohash and all neighbors (9 cells total)
pub fn center_and_neighbors(hash: &str) -> Vec<String> {
    let mut cells = vec![hash.to_string()];
    cells.extend(neighbors(hash));
    cells
}

/// Expand outward from `hash` by `rings` rings of neighbours.
///
/// `rings == 0` yields just the centre cell; `rings == 1` yields the 3x3 Moore
/// neighbourhood (identical to [`center_and_neighbors`]); `rings == n` yields up
/// to `(2n+1)^2` cells. Duplicates are removed and insertion order is preserved
/// so the centre cell is scanned first (helpful for KNN, which wants the nearest
/// candidates early).
///
/// Implemented as a breadth-first walk over `geohash::neighbors`, because the
/// geohash crate offers no direct k-ring primitive. Cells at the poles or across
/// the antimeridian may have fewer than eight neighbours; those are simply
/// absent rather than an error, which is correct — a missing neighbour is a cell
/// that does not exist.
pub fn ring_expand(hash: &str, rings: usize) -> Vec<String> {
    if hash.is_empty() {
        return Vec::new();
    }

    let mut out = vec![hash.to_string()];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(hash.to_string());
    let mut frontier = vec![hash.to_string()];

    for _ in 0..rings {
        let mut next = Vec::new();
        for cell in &frontier {
            for n in neighbors(cell) {
                if seen.insert(n.clone()) {
                    out.push(n.clone());
                    next.push(n);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    out
}

/// Generate multi-precision geohashes for a point at the given precisions.
pub fn multi_precision_geohashes_at(
    lon: f64,
    lat: f64,
    precisions: &[usize],
) -> Vec<(usize, String)> {
    precisions
        .iter()
        .filter_map(|&precision| encode_point(lon, lat, precision).map(|h| (precision, h)))
        .collect()
}

/// Generate multi-precision geohashes for a point at the default precisions.
pub fn multi_precision_geohashes(lon: f64, lat: f64) -> Vec<(usize, String)> {
    multi_precision_geohashes_at(lon, lat, INDEX_PRECISIONS)
}

/// Calculate the approximate search radius for a geohash precision
pub fn precision_radius_meters(precision: usize) -> f64 {
    match precision {
        1 => 5_000_000.0,
        2 => 1_250_000.0,
        3 => 156_000.0,
        4 => 39_000.0,
        5 => 4_900.0,
        6 => 1_200.0,
        7 => 153.0,
        8 => 38.0,
        9 => 4.8,
        10 => 1.2,
        11 => 0.15,
        12 => 0.04,
        _ => 5_000_000.0,
    }
}

/// Choose the finest geohash precision whose cell still covers `radius_meters`,
/// restricted to precisions the index was actually built at.
///
/// Returns `None` when no member of `precisions` is coarse enough — the caller
/// must then ring-expand at the coarsest available precision (see
/// [`super::plan_radius_scan`], which is the only correct entry point).
///
/// The old unrestricted `precision_for_radius` walked 12 -> 1 over precisions
/// that may never have been indexed. Combined with the `\0`-terminated geohash
/// prefix (which matches exactly one precision), that returned zero rows outside
/// a narrow radius band. Do not reintroduce it.
pub fn precision_for_radius_in(radius_meters: f64, precisions: &[usize]) -> Option<usize> {
    precisions
        .iter()
        .copied()
        .filter(|&p| precision_radius_meters(p) >= radius_meters)
        .max()
}
