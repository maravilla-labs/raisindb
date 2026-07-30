//! Geohash generation for GeoJSON geometries.
//!
//! The centroid/bbox/cover computation lives in [`super::cover`]; this module
//! keeps the thin default-policy wrappers the older call sites use.

use super::cover::cells_for_geometry;
use raisin_models::nodes::properties::{GeoJson, SpatialPolicy};

/// Generate all geohashes for indexing a geometry under the default policy.
///
/// Prefer [`cells_for_geometry`] with an explicit
/// [`SpatialPolicy`] — it also returns the bbox and altitude extent the index
/// value record needs, computed in the same pass, **and it surfaces an
/// un-normalisable SRID as an error**. This convenience wrapper flattens that
/// error to an empty cell list, which is right for a diagnostic caller and wrong
/// for a write path; no write path uses it.
pub fn geohashes_for_geometry(geojson: &GeoJson) -> Vec<String> {
    cells_for_geometry(geojson, &SpatialPolicy::default())
        .unwrap_or_default()
        .map(|c| c.cells)
        .unwrap_or_default()
}
