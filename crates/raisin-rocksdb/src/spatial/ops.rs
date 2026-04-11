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

/// Generate multi-precision geohashes for a point
pub fn multi_precision_geohashes(lon: f64, lat: f64) -> Vec<(usize, String)> {
    INDEX_PRECISIONS
        .iter()
        .filter_map(|&precision| encode_point(lon, lat, precision).map(|h| (precision, h)))
        .collect()
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

/// Choose the optimal geohash precision for a given search radius
pub fn precision_for_radius(radius_meters: f64) -> usize {
    for precision in (1..=12).rev() {
        if precision_radius_meters(precision) >= radius_meters {
            return precision;
        }
    }
    12
}

/// Generate geohash cells to scan for a proximity query
pub fn cells_for_radius(center_lon: f64, center_lat: f64, radius_meters: f64) -> Vec<String> {
    let precision = precision_for_radius(radius_meters);
    match encode_point(center_lon, center_lat, precision) {
        Some(center_hash) => center_and_neighbors(&center_hash),
        None => Vec::new(),
    }
}
