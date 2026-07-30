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

//! Coordinate reference systems and reprojection for RaisinDB.
//!
//! # Why this crate exists
//!
//! RaisinDB used to hardcode `ST_SRID(...) = 4326` and had no `ST_TRANSFORM` at
//! all. This crate makes SRID *honest*: a geometry's CRS is data, and converting
//! between CRS is a real computation that either succeeds or fails loudly.
//!
//! # The layered backend model
//!
//! Coverage grows with Cargo features; a default build needs **no system
//! libraries at all**.
//!
//! | Tier | Feature | Coverage | Build prerequisites |
//! |------|---------|----------|---------------------|
//! | 1 | *(always on)* | EPSG:4326, 3857, all 120 WGS84 UTM zones | none |
//! | 2 | `proj4rs-backend` | ~1000 EPSG codes, pure Rust, WASM-safe | none |
//! | 3 | `proj-backend` | full EPSG database + datum grids | libproj >= 9.6 (pkg-config) |
//! | 3b | `proj-backend-bundled` | as tier 3, libproj built from source | C/C++ toolchain, sqlite3, libtiff |
//!
//! Tiers are tried in order of *fidelity*, highest first, so enabling a wider
//! backend transparently improves accuracy for codes the built-in also knows.
//!
//! # The one hard guarantee
//!
//! [`transform_coord`] never returns approximate or untransformed coordinates as
//! a fallback. If no compiled backend can do the job it returns
//! [`ProjError::UnsupportedTransform`], whose message names the Cargo feature
//! that would enable it. Silent passthrough was the original bug; it is not a
//! behaviour this crate has.
//!
//! # Examples
//!
//! ```
//! use raisin_proj::{Crs, transform_coord};
//!
//! // Zurich, WGS84 lon/lat -> WebMercator metres.
//! let (x, y) = transform_coord(Crs::WGS84, Crs::WEB_MERCATOR, 8.54, 47.37).unwrap();
//! assert!((x - 950_668.45).abs() < 0.01);
//!
//! // An SRID nothing is compiled for fails loudly instead of lying.
//! //
//! // 999999 is not an assigned EPSG code, so this holds under *every* feature
//! // combination. Do not use a real-but-obscure code here (EPSG:31370 was the
//! // original choice): `proj4rs-backend` knows ~1000 codes and `proj-backend`
//! // knows the whole database, so a real code makes this example's outcome
//! // depend on the enabled features.
//! let err = transform_coord(Crs::WGS84, Crs::from_srid(999_999), 4.35, 50.85)
//!     .unwrap_err();
//! // The message names the feature that would widen coverage.
//! assert!(err.to_string().contains("proj"));
//! ```

pub mod backends;
pub mod builtin;
pub mod crs;
pub mod error;
pub mod geometry;

pub use crs::{Crs, EPSG_WEB_MERCATOR, EPSG_WGS84};
pub use error::{Backend, ProjError, Result};
pub use geometry::transform_geometry;

/// Transform one coordinate from `from` to `to`.
///
/// Coordinate order is always **x, y** — meaning `(longitude, latitude)` for
/// geographic CRS such as EPSG:4326, matching GeoJSON (RFC 7946 §3.1.1) and the
/// rest of RaisinDB. It is *not* the `(lat, lon)` order some GIS tools use.
///
/// # Errors
///
/// * [`ProjError::UnsupportedTransform`] — no compiled backend handles the pair.
///   The message names the feature to enable.
/// * [`ProjError::OutOfDomain`] — the point has no image in the target CRS
///   (e.g. a pole in WebMercator).
/// * [`ProjError::BackendFailure`] — a backend was invoked and failed.
pub fn transform_coord(from: Crs, to: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    if from == to {
        return Ok((x, y));
    }
    backends::transform(from, to, x, y)
}

/// Report which backends this build actually contains, most capable first.
///
/// Exposed so an operator can see the active projection capability at runtime
/// (e.g. in a `/management` health payload) rather than discovering it from a
/// failed query.
// Built with cfg-gated pushes rather than a literal, so the order stays
// "most capable first" for whichever subset is compiled in.
#[allow(clippy::vec_init_then_push)]
pub fn available_backends() -> Vec<Backend> {
    let mut v = Vec::new();
    #[cfg(feature = "proj-backend")]
    v.push(Backend::Proj);
    #[cfg(feature = "proj4rs-backend")]
    v.push(Backend::Proj4rs);
    v.push(Backend::Builtin);
    v
}

/// True when this build can transform between `from` and `to`.
///
/// Cheap enough to call during query planning to decide whether an
/// `ST_TRANSFORM` is satisfiable before executing a scan.
pub fn can_transform(from: Crs, to: Crs) -> bool {
    from == to || backends::can_transform(from, to)
}

/// Version of the write-time index normaliser ([`normalize_for_index`]).
///
/// Bump this whenever the tier-1 maths changes. It is folded into the spatial
/// index's policy fingerprint, so a change forces a reindex instead of silently
/// mixing entries produced under two different conventions.
///
/// **Must stay equal to `raisin_models::nodes::properties::SPATIAL_NORMALIZER_VERSION`.**
/// The constant is duplicated because `raisin-models` is depended on by every
/// crate — including the thin clients — and must not pull in `geo`.
/// `raisin-geometry` depends on both and asserts the equality in a test.
pub const NORMALIZER_VERSION: u32 = 1;

/// True when `srid` can be converted to WGS84 in **every** build of RaisinDB,
/// whatever Cargo features are enabled.
///
/// That is exactly the built-in tier: EPSG:4326, EPSG:3857 (and its 3785/900913
/// aliases), and all 120 WGS84 UTM zones. Feature-independent by construction.
pub fn is_index_normalizable(srid: Crs) -> bool {
    builtin::supports(srid)
}

/// Convert a coordinate to WGS84 lon/lat **for spatial index keys**, using the
/// built-in tier only.
///
/// # Why this exists as a separate function from [`transform_coord`]
///
/// RaisinDB is a masterless multi-master cluster in which every node builds its
/// own local spatial index from replicated records. Geohash cells are defined on
/// lon/lat degrees, so index keys must be normalised to WGS84 at write time —
/// and if that normalisation used "whatever backend happens to be compiled in",
/// a node built with `proj-backend` would index an EPSG:31370 geometry while a
/// node without it would not. Same replicated data, divergent local indexes, and
/// the same query returning different answers depending on which node replies.
/// In a masterless cluster that is forbidden.
///
/// So this function **must not consult `proj4rs` or `libproj` even when they are
/// compiled in**. That restriction is the whole point; it is what makes "node A
/// and node B produce identical index bytes" a structural property rather than a
/// deployment convention. A test under `--features proj-backend` asserts it.
///
/// A query-time [`transform_coord`] is deliberately *not* restricted this way: a
/// query result is per-request, not shared state, so widening coverage there is
/// a pure improvement.
///
/// # Errors
///
/// [`ProjError::UnsupportedSrid`] when the SRID is outside the built-in tier —
/// loudly, at write time, before an unindexable value can be stored. Storing a
/// geometry that no index can see would make `ST_DWITHIN` miss rows forever with
/// no signal, which is the exact failure class this design removes.
pub fn normalize_for_index(srid: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    if !is_index_normalizable(srid) {
        return Err(ProjError::UnsupportedSrid {
            srid: srid.srid(),
            active: builtin::BACKEND_NAME,
            suggestion: "ST_TRANSFORM(..., 4326) at write time, or store in EPSG:4326, \
                         EPSG:3857 or a WGS84 UTM zone",
        });
    }
    // Same domain guard every transform gets, so a pole or a NaN cannot geohash
    // to a garbage cell.
    backends::check_input_domain(srid, Crs::WGS84, x, y)?;
    builtin::transform(srid, Crs::WGS84, x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_is_always_available() {
        assert!(available_backends().contains(&Backend::Builtin));
    }

    #[test]
    fn identity_short_circuits_for_unknown_srid() {
        let odd = Crs::from_srid(999_999);
        assert_eq!(transform_coord(odd, odd, 1.0, 2.0).unwrap(), (1.0, 2.0));
        assert!(can_transform(odd, odd));
    }

    #[test]
    fn index_normalizable_set_is_the_builtin_tier() {
        assert!(is_index_normalizable(Crs::WGS84));
        assert!(is_index_normalizable(Crs::WEB_MERCATOR));
        assert!(is_index_normalizable(Crs::from(900_913)));
        assert!(is_index_normalizable(Crs::from_srid(32632)));
        assert!(is_index_normalizable(Crs::from_srid(32756)));
        // Belgian Lambert 72 — a real CRS the built-in tier does not own.
        assert!(!is_index_normalizable(Crs::from_srid(31_370)));
    }

    #[test]
    fn normalize_for_index_is_identity_on_wgs84_and_exact_on_mercator() {
        assert_eq!(
            normalize_for_index(Crs::WGS84, 8.54, 47.37).unwrap(),
            (8.54, 47.37)
        );

        let (x, y) = transform_coord(Crs::WGS84, Crs::WEB_MERCATOR, 8.54, 47.37).unwrap();
        let (lon, lat) = normalize_for_index(Crs::WEB_MERCATOR, x, y).unwrap();
        assert!((lon - 8.54).abs() < 1e-9, "{lon}");
        assert!((lat - 47.37).abs() < 1e-9, "{lat}");
    }

    /// THE cluster-determinism guarantee. Run this under `--features
    /// proj4rs-backend` and `--features proj-backend` too: enabling a wider
    /// backend must NOT widen what the index normaliser accepts, or two nodes
    /// with different builds would hold different index entries for the same
    /// replicated record.
    #[test]
    fn normalize_for_index_never_uses_an_optional_backend() {
        // EPSG:31370 is known to crs-definitions (tier 2) and to libproj
        // (tier 3), but not to the built-in tier.
        let err = normalize_for_index(Crs::from_srid(31_370), 150_000.0, 170_000.0).unwrap_err();
        assert!(
            matches!(err, ProjError::UnsupportedSrid { .. }),
            "must refuse regardless of enabled features, got {err:?}"
        );

        // The generic transform, by contrast, is free to use whatever is
        // compiled in — so on a wide build these two disagree, deliberately.
        if available_backends().len() > 1 {
            assert!(can_transform(Crs::WGS84, Crs::from_srid(31_370)));
        }
    }

    #[test]
    fn normalize_for_index_rejects_nonsense_before_it_can_be_geohashed() {
        for (x, y) in [(f64::NAN, 0.0), (0.0, 91.0), (0.0, f64::INFINITY)] {
            assert!(
                matches!(
                    normalize_for_index(Crs::WGS84, x, y),
                    Err(ProjError::OutOfDomain { .. })
                ),
                "({x}, {y}) must not reach the geohash encoder"
            );
        }
    }

    #[test]
    fn web_mercator_alias_is_not_a_real_transform() {
        // 900913 normalizes to 3857, so this is identity, not an error.
        let a = Crs::from(900_913);
        let b = Crs::from(3857);
        assert_eq!(a, b);
        assert_eq!(transform_coord(a, b, 5.0, 6.0).unwrap(), (5.0, 6.0));
    }
}
