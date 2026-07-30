// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The single conversion layer between RaisinDB's geometry model, `serde_json`
//! and georust `geo-types`.
//!
//! # Why this is a crate of its own
//!
//! Before it existed, `raisin-sql-execution` had three hand-rolled converters —
//! for `Point`, `LineString` and `Polygon` — and **that**, not a missing library,
//! is why `Multi*` and `GeometryCollection` were unsupported across all 49 ST_\*
//! functions, why every topological predicate had a different incomplete set of
//! type pairs, and why several of them silently returned `false` for inputs they
//! did not recognise.
//!
//! It is a crate rather than a module because two things must be true at once:
//! `raisin-models` has to stay dependency-light (every crate imports it,
//! including the thin clients, so it must not pull in `geo`), and
//! `raisin-sql-execution` and `raisin-rocksdb` have to share *one*
//! implementation. Anywhere less central invites a fourth converter.
//!
//! # The three representations, and which conversion to use
//!
//! | From | To | Function | Notes |
//! |------|----|----------|-------|
//! | [`GeoJson`] model | `geo` | [`to_geo_from_model`] | serde-free; the storage/index hot path |
//! | `geo` | [`GeoJson`] model | [`to_model`] | |
//! | `serde_json::Value` | `geo` | [`to_geo`] | via the `geojson` crate; the SQL evaluator path |
//! | `geo` | `serde_json::Value` | [`from_geo`] | |
//!
//! # Three invariants every caller depends on
//!
//! 1. **Axis order is pinned to `(x, y)` = `(longitude, latitude)`**, everywhere,
//!    with no exceptions and no per-EPSG-code axis flipping. This diverges from
//!    the EPSG authority (which defines EPSG:4326 as lat/lon) and matches
//!    GeoJSON RFC 7946 §3.1.1, PostGIS, `geo-types`, the `geohash` crate and
//!    every web mapping library. Honouring authority order instead would break
//!    interop with all of them and with our own stored data.
//! 2. **Z is projected away at this boundary.** `geo_types::Coord` is strictly
//!    2-D. Altitude survives as [`Geom::z_range`], and Z-aware functions read it
//!    from there or from the JSON via [`zdim`]. Every other ST_\* function
//!    ignores Z, exactly as PostGIS's 2-D predicates do.
//! 3. **Emptiness propagates, it does not error.** There is one canonical empty
//!    geometry ([`EMPTY_GEOMETRY_JSON`]) and every function accepts it.
//!    Undefined emptiness was the largest source of spurious "unsupported
//!    geometry type" errors in the previous implementation.
//!
//! # Examples
//!
//! ```
//! use raisin_geometry::{to_geo, from_geo};
//!
//! // A type the old three hand-rolled converters could not touch.
//! let value = serde_json::json!({
//!     "type": "MultiPolygon",
//!     "coordinates": [[[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]]]]
//! });
//! let geom = to_geo(&value, None).unwrap();
//! assert!(matches!(geom.geometry, geo::Geometry::MultiPolygon(_)));
//! assert_eq!(from_geo(&geom).unwrap(), value);
//! ```

pub mod error;
pub mod geom;
pub mod json;
pub mod metric;
pub mod model;
pub mod srid;
pub mod zdim;

pub use error::{GeometryError, Result};
pub use geom::{empty, envelope_of, is_empty, Envelope, Geom, EMPTY_GEOMETRY_JSON};
pub use json::{from_geo, to_geo};
pub use metric::{from_metric, to_metric, transform};
pub use model::{to_geo_from_model, to_model};
pub use srid::{require_same_srid, resolve_pair_srid, srid_member, srid_of, with_srid};
pub use zdim::{force_2d, force_3d, ndims, z_gap, z_of_point};

/// Re-exported so callers need not add a `raisin-proj` dependency just to name a
/// CRS in a signature.
pub use raisin_proj::{Crs, ProjError};

/// The mean Earth radius used for every geodesic measurement, in metres.
///
/// This is `geo`'s `HaversineMeasure::GRS80_MEAN_RADIUS`, and it is *also* the
/// constant the spatial index's own Haversine post-filter uses. Sharing it is
/// what makes the index's distance filter and `ST_DISTANCE` agree to the metre
/// instead of disagreeing by a few hundred parts per million — a disagreement
/// that shows up as a row appearing or vanishing right at the radius boundary.
pub const EARTH_MEAN_RADIUS_METERS: f64 = 6_371_008.8;

#[cfg(test)]
mod tests {
    use super::*;

    /// `raisin-models` cannot depend on `raisin-proj` (it must not pull in `geo`),
    /// so the normaliser version is declared in both. This crate depends on both
    /// and is where the duplication is checked.
    #[test]
    fn normalizer_version_agrees_across_crates() {
        assert_eq!(
            raisin_models::nodes::properties::SPATIAL_NORMALIZER_VERSION,
            raisin_proj::NORMALIZER_VERSION,
            "bump both constants together, or a policy fingerprint means two \
             different things on two nodes"
        );
    }

    /// The index post-filter and `ST_DISTANCE` must use the same sphere, or rows
    /// flicker in and out right at the radius boundary. Verified rather than
    /// trusted.
    #[test]
    fn earth_radius_matches_geos_haversine_measure() {
        use geo::{Distance, Haversine, HaversineMeasure, Point};
        let a = Point::new(0.0, 0.0);
        let b = Point::new(1.0, 0.0);
        let ours: f64 = HaversineMeasure::new(EARTH_MEAN_RADIUS_METERS).distance(a, b);
        let theirs: f64 = Haversine.distance(a, b);
        assert!(
            (ours - theirs).abs() < 1e-6,
            "our radius gives {ours} m, geo's default gives {theirs} m"
        );
    }

    /// End-to-end across both directions and both representations, for a value
    /// that uses every extension at once: Multi\*, altitude and a projected SRID.
    #[test]
    fn model_and_json_paths_agree() {
        let value = serde_json::json!({
            "type": "MultiLineString",
            "coordinates": [[[2683000.0, 1247000.0, 400.0], [2683010.0, 1247010.0]]],
            "srid": 32632
        });

        let via_json = to_geo(&value, None).unwrap();
        let model: raisin_models::nodes::properties::GeoJson =
            serde_json::from_value(value.clone()).unwrap();
        let via_model = to_geo_from_model(&model, None).unwrap();

        assert_eq!(via_json, via_model, "the two entry points must agree");
        assert_eq!(via_json.srid, Crs::from_srid(32632));
        assert_eq!(via_json.z_range, Some((400.0, 400.0)));

        // Round trip back out; altitude is the documented one-way loss.
        let out = from_geo(&via_json).unwrap();
        assert_eq!(out["type"], "MultiLineString");
        assert_eq!(out["srid"], 32632);
        assert_eq!(
            out["coordinates"][0][0],
            serde_json::json!([2683000.0, 1247000.0])
        );
    }

    #[test]
    fn a_geometry_error_becomes_a_validation_error_not_an_internal_one() {
        let err: raisin_error::Error = to_geo(&serde_json::json!({"type": "Nope"}), None)
            .unwrap_err()
            .into();
        assert!(
            matches!(err, raisin_error::Error::Validation(_)),
            "a malformed geometry is bad input, not a backend fault: {err:?}"
        );
    }
}
