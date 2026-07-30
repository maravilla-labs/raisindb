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

//! Pure-Rust broad-EPSG backend, built on `proj4rs` + `crs-definitions`.
//!
//! Enabled by the `proj4rs-backend` feature. No C toolchain and no system
//! libraries, so it also works under WASM.
//!
//! # Two traps this module exists to contain
//!
//! 1. **`proj4rs` speaks radians**, not degrees, for any geographic CRS. Every
//!    angular value crossing this boundary is converted. Forgetting this yields
//!    coordinates wrong by a factor of ~57.3 that still look like plausible
//!    numbers.
//! 2. **`proj4rs` runs in "relaxed mode" by default and returns `NaN` instead of
//!    an error when a projection fails.** A `NaN` propagated into the spatial
//!    index would poison comparisons silently, so every result is checked for
//!    finiteness here and turned into a real error.
//!
//! `proj4rs` also documents nadgrid (datum shift) support as experimental. For
//! datum-accurate work prefer the `proj-backend` feature.

use proj4rs::Proj;

use crate::crs::Crs;
use crate::error::{ProjError, Result};

/// Name used in diagnostics for this backend.
pub const BACKEND_NAME: &str = "proj4rs";

/// Build a `proj4rs::Proj` for an EPSG code, if `crs-definitions` knows it.
fn projection(crs: Crs) -> Option<Proj> {
    let code = u16::try_from(crs.srid()).ok()?;
    Proj::from_epsg_code(code).ok()
}

/// Whether this backend has a definition for `crs`.
///
/// Note `crs-definitions` is keyed by `u16`, so EPSG codes above 65535 (some
/// ESRI-originated and IGNF codes) are outside its reach regardless.
pub fn supports(crs: Crs) -> bool {
    projection(crs).is_some()
}

/// Transform one coordinate. Angular CRS are converted to/from radians here.
pub fn transform(from: Crs, to: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    let src = projection(from).ok_or(ProjError::UnsupportedSrid {
        srid: from.srid(),
        active: BACKEND_NAME,
        suggestion: "proj-backend",
    })?;
    let dst = projection(to).ok_or(ProjError::UnsupportedSrid {
        srid: to.srid(),
        active: BACKEND_NAME,
        suggestion: "proj-backend",
    })?;

    // Degrees -> radians on the way in, if the source is angular.
    let mut point = if src.is_latlong() {
        (x.to_radians(), y.to_radians(), 0.0_f64)
    } else {
        (x, y, 0.0_f64)
    };

    proj4rs::transform::transform(&src, &dst, &mut point).map_err(|e| {
        ProjError::BackendFailure {
            backend: BACKEND_NAME,
            from: from.srid(),
            to: to.srid(),
            message: e.to_string(),
        }
    })?;

    // Radians -> degrees on the way out, if the target is angular.
    let (mut out_x, mut out_y) = (point.0, point.1);
    if dst.is_latlong() {
        out_x = out_x.to_degrees();
        out_y = out_y.to_degrees();
    }

    // Relaxed mode signals failure with NaN rather than Err. Refuse to return it.
    if !out_x.is_finite() || !out_y.is_finite() {
        return Err(ProjError::OutOfDomain {
            x,
            y,
            srid: to.srid(),
            reason:
                "proj4rs produced a non-finite coordinate (point outside the projection domain)",
        });
    }

    Ok((out_x, out_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrees_with_builtin_on_web_mercator() {
        // Cross-validation: two independent implementations of the same
        // projection must agree, which is the strongest check available here.
        let ours = crate::builtin::mercator::wgs84_to_web_mercator(8.54, 47.37).unwrap();
        let theirs = transform(Crs::WGS84, Crs::WEB_MERCATOR, 8.54, 47.37).unwrap();
        assert!((ours.0 - theirs.0).abs() < 1e-3, "{ours:?} vs {theirs:?}");
        assert!((ours.1 - theirs.1).abs() < 1e-3, "{ours:?} vs {theirs:?}");
    }

    #[test]
    fn agrees_with_builtin_on_utm() {
        let ours = crate::builtin::utm::wgs84_to_utm(8.54, 47.37, 32, true, 32632).unwrap();
        let theirs = transform(Crs::WGS84, Crs::from_srid(32632), 8.54, 47.37).unwrap();
        // 1 cm tolerance: proj4rs and our Krüger series truncate differently.
        assert!((ours.0 - theirs.0).abs() < 0.01, "{ours:?} vs {theirs:?}");
        assert!((ours.1 - theirs.1).abs() < 0.01, "{ours:?} vs {theirs:?}");
    }

    #[test]
    fn covers_a_crs_the_builtin_cannot() {
        // EPSG:31370, Belgian Lambert 72 — the case that fails on a default build.
        assert!(supports(Crs::from_srid(31_370)));
        let (x, y) = transform(Crs::WGS84, Crs::from_srid(31_370), 4.35, 50.85).unwrap();
        // Brussels in Lambert 72 is roughly (149000, 170000).
        assert!((140_000.0..160_000.0).contains(&x), "easting {x}");
        assert!((160_000.0..180_000.0).contains(&y), "northing {y}");
    }

    #[test]
    fn round_trips_through_a_projected_crs() {
        let (x, y) = transform(Crs::WGS84, Crs::from_srid(31_370), 4.35, 50.85).unwrap();
        let (lon, lat) = transform(Crs::from_srid(31_370), Crs::WGS84, x, y).unwrap();
        assert!((lon - 4.35).abs() < 1e-6, "lon {lon}");
        assert!((lat - 50.85).abs() < 1e-6, "lat {lat}");
    }
}
