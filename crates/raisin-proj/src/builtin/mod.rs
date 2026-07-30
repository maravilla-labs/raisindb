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

//! The always-available, zero-dependency projection backend.
//!
//! Covers the transforms that dominate real workloads:
//! * EPSG:4326 <-> EPSG:3857 (every web map)
//! * EPSG:4326 <-> EPSG:326xx / 327xx (all 120 WGS84 UTM zones)
//! * any CRS to itself (identity)
//! * any pair of the above, by routing through WGS84 as a hub
//!
//! Anything else returns [`ProjError::UnsupportedTransform`] naming the feature
//! that would handle it. It never returns the input unchanged.

pub mod mercator;
pub mod utm;

use crate::crs::Crs;
use crate::error::{ProjError, Result};

/// Name used in diagnostics for this backend.
pub const BACKEND_NAME: &str = "builtin";

/// True when this backend can convert `crs` to/from WGS84 lon/lat.
pub fn supports(crs: Crs) -> bool {
    crs.is_geographic() || crs == Crs::WEB_MERCATOR || crs.utm_zone().is_some()
}

/// Convert a coordinate in `from` into WGS84 lon/lat degrees.
fn to_wgs84(from: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    if from.is_geographic() {
        return Ok((x, y));
    }
    if from == Crs::WEB_MERCATOR {
        return mercator::web_mercator_to_wgs84(x, y);
    }
    if let Some((zone, north)) = from.utm_zone() {
        return utm::utm_to_wgs84(x, y, zone, north, from.srid());
    }
    Err(unsupported(from, Crs::WGS84))
}

/// Convert WGS84 lon/lat degrees into `to`.
fn from_wgs84(to: Crs, lon: f64, lat: f64) -> Result<(f64, f64)> {
    if to.is_geographic() {
        return Ok((lon, lat));
    }
    if to == Crs::WEB_MERCATOR {
        return mercator::wgs84_to_web_mercator(lon, lat);
    }
    if let Some((zone, north)) = to.utm_zone() {
        return utm::wgs84_to_utm(lon, lat, zone, north, to.srid());
    }
    Err(unsupported(Crs::WGS84, to))
}

/// Transform a single coordinate between two CRS this backend understands.
///
/// WGS84 acts as the hub, so e.g. UTM 32N -> WebMercator works by unprojecting
/// then reprojecting. Both legs are sub-millimetre, so the composition is too.
pub fn transform(from: Crs, to: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    if from == to {
        return Ok((x, y));
    }
    if !supports(from) || !supports(to) {
        return Err(unsupported(from, to));
    }
    let (lon, lat) = to_wgs84(from, x, y)?;
    from_wgs84(to, lon, lat)
}

/// Build the error for a pair this backend cannot bridge, naming the feature
/// that would. The suggestion escalates: prefer the pure-Rust backend, and only
/// mention the C one when it is the sole remaining option.
fn unsupported(from: Crs, to: Crs) -> ProjError {
    ProjError::UnsupportedTransform {
        from: from.srid(),
        to: to.srid(),
        suggestion: SUGGESTED_FEATURE,
    }
}

/// The feature an operator should enable next, given what is already compiled in.
#[cfg(not(any(feature = "proj4rs-backend", feature = "proj-backend")))]
pub const SUGGESTED_FEATURE: &str = "proj4rs-backend (pure Rust) or proj-backend (needs libproj)";
#[cfg(all(feature = "proj4rs-backend", not(feature = "proj-backend")))]
pub const SUGGESTED_FEATURE: &str = "proj-backend (needs libproj >= 9.6)";
#[cfg(feature = "proj-backend")]
pub const SUGGESTED_FEATURE: &str = "proj-backend-bundled, or install a newer libproj";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_exact_for_any_srid() {
        // Even an SRID no backend knows: identity must never error, and must not
        // perturb the coordinate.
        let odd = Crs::from_srid(31_370);
        assert_eq!(transform(odd, odd, 1.5, 2.5).unwrap(), (1.5, 2.5));
    }

    #[test]
    fn routes_utm_to_web_mercator_through_wgs84() {
        // Zurich in UTM 32N -> WebMercator, compared against going direct from
        // WGS84. The hub route must agree to well under a millimetre.
        let (e, n) = utm::wgs84_to_utm(8.54, 47.37, 32, true, 32632).unwrap();
        let via_hub = transform(Crs::from_srid(32632), Crs::WEB_MERCATOR, e, n).unwrap();
        let direct = mercator::wgs84_to_web_mercator(8.54, 47.37).unwrap();
        assert!(
            (via_hub.0 - direct.0).abs() < 1e-3,
            "{via_hub:?} {direct:?}"
        );
        assert!(
            (via_hub.1 - direct.1).abs() < 1e-3,
            "{via_hub:?} {direct:?}"
        );
    }

    #[test]
    fn unknown_srid_errors_and_names_a_feature() {
        // EPSG:31370 (Belgian Lambert 72) is not built in.
        let err = transform(Crs::WGS84, Crs::from_srid(31_370), 4.35, 50.85).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("31370"), "{msg}");
        // The whole point: it tells the operator what to do.
        assert!(msg.contains("backend"), "{msg}");
    }

    #[test]
    fn supports_reports_builtin_coverage() {
        assert!(supports(Crs::WGS84));
        assert!(supports(Crs::WEB_MERCATOR));
        assert!(supports(Crs::from_srid(32632)));
        assert!(supports(Crs::from_srid(32756)));
        assert!(!supports(Crs::from_srid(31_370)));
    }
}
