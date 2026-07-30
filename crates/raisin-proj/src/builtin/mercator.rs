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

//! EPSG:4326 <-> EPSG:3857 (WGS84 / Pseudo-Mercator).
//!
//! This is a *closed form* projection, not a series approximation: EPSG:3857 is
//! defined as a spherical Mercator evaluated on the WGS84 semi-major axis, so
//! these two functions are exact inverses of the authoritative definition and
//! agree with PROJ to floating-point precision.

use crate::crs::EPSG_WEB_MERCATOR;
use crate::error::{ProjError, Result};

/// WGS84 semi-major axis in metres. EPSG:3857 uses this as the sphere radius.
pub const WGS84_SEMI_MAJOR: f64 = 6_378_137.0;

/// Northern/southern limit of the EPSG:3857 domain in degrees.
///
/// Mercator sends the poles to infinity, so the CRS is only defined up to
/// `atan(sinh(pi))` in degrees — the latitude whose northing equals the easting
/// extent, making the projected world square.
///
/// Kept at full published precision even though `f64` cannot represent the last
/// digit, so the constant is checkable against the EPSG:3857 spec by eye.
#[allow(clippy::excessive_precision)]
pub const WEB_MERCATOR_LAT_LIMIT: f64 = 85.051_128_779_806_604;

/// Half the circumference at the equator — the ±X and ±Y extent of EPSG:3857.
pub const WEB_MERCATOR_EXTENT: f64 = std::f64::consts::PI * WGS84_SEMI_MAJOR;

/// Project WGS84 lon/lat degrees to EPSG:3857 metres.
///
/// Latitudes beyond [`WEB_MERCATOR_LAT_LIMIT`] are rejected rather than clamped:
/// clamping would move the point, which is precisely the silent-inaccuracy
/// behaviour this crate exists to avoid.
pub fn wgs84_to_web_mercator(lon: f64, lat: f64) -> Result<(f64, f64)> {
    if !lon.is_finite() || !lat.is_finite() {
        return Err(ProjError::OutOfDomain {
            x: lon,
            y: lat,
            srid: EPSG_WEB_MERCATOR,
            reason: "coordinates must be finite",
        });
    }
    if lat.abs() > WEB_MERCATOR_LAT_LIMIT {
        return Err(ProjError::OutOfDomain {
            x: lon,
            y: lat,
            srid: EPSG_WEB_MERCATOR,
            reason: "latitude exceeds the ±85.0511287798066° Mercator limit",
        });
    }

    let x = WGS84_SEMI_MAJOR * lon.to_radians();
    let lat_rad = lat.to_radians();
    // ln(tan(pi/4 + phi/2)) written as asinh(tan(phi)) for better conditioning
    // near the equator; the two are algebraically identical.
    let y = WGS84_SEMI_MAJOR * lat_rad.tan().asinh();
    Ok((x, y))
}

/// Unproject EPSG:3857 metres back to WGS84 lon/lat degrees.
pub fn web_mercator_to_wgs84(x: f64, y: f64) -> Result<(f64, f64)> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ProjError::OutOfDomain {
            x,
            y,
            srid: EPSG_WEB_MERCATOR,
            reason: "coordinates must be finite",
        });
    }

    let lon = (x / WGS84_SEMI_MAJOR).to_degrees();
    let lat = (y / WGS84_SEMI_MAJOR).sinh().atan().to_degrees();
    Ok((lon, lat))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values computed independently from the authoritative EPSG:3857
    /// definition (spherical Mercator on the WGS84 semi-major axis). They are
    /// cross-checkable against `cs2cs EPSG:4326 EPSG:3857` on any PROJ install.
    /// Coordinate order here is lon/lat in, x/y out.
    const CASES: &[(f64, f64, f64, f64)] = &[
        // Null island.
        (0.0, 0.0, 0.0, 0.0),
        // Zurich.
        (8.54, 47.37, 950_668.451_374_556_3, 6_002_677.997_532_714),
        // Sydney.
        (
            151.209_295,
            -33.868_15,
            16_832_541.722_609_885,
            -4_011_111.502_890_065_3,
        ),
        // Quito, essentially on the equator.
        (
            -78.467_838,
            -0.180_653,
            -8_734_999.769_809_082,
            -20_110.233_290_761_88,
        ),
    ];

    #[test]
    fn matches_reference_values() {
        for &(lon, lat, ex, ey) in CASES {
            let (x, y) = wgs84_to_web_mercator(lon, lat).unwrap();
            // Sub-millimetre agreement.
            assert!(
                (x - ex).abs() < 1e-3,
                "x for ({lon},{lat}): got {x}, want {ex}"
            );
            assert!(
                (y - ey).abs() < 1e-3,
                "y for ({lon},{lat}): got {y}, want {ey}"
            );
        }
    }

    #[test]
    fn round_trips_to_nanometre_precision() {
        for &(lon, lat, _, _) in CASES {
            let (x, y) = wgs84_to_web_mercator(lon, lat).unwrap();
            let (lon2, lat2) = web_mercator_to_wgs84(x, y).unwrap();
            assert!((lon - lon2).abs() < 1e-12, "lon {lon} -> {lon2}");
            assert!((lat - lat2).abs() < 1e-12, "lat {lat} -> {lat2}");
        }
    }

    #[test]
    fn extent_corner_is_consistent() {
        let (x, y) = wgs84_to_web_mercator(180.0, WEB_MERCATOR_LAT_LIMIT).unwrap();
        assert!((x - WEB_MERCATOR_EXTENT).abs() < 1e-6);
        assert!((y - WEB_MERCATOR_EXTENT).abs() < 1e-3);
    }

    #[test]
    fn rejects_poles_rather_than_clamping() {
        // The pole has no Mercator image. Returning a clamped value would be a
        // silently wrong coordinate, so this must be an error.
        assert!(wgs84_to_web_mercator(0.0, 90.0).is_err());
        assert!(wgs84_to_web_mercator(0.0, -90.0).is_err());
        assert!(wgs84_to_web_mercator(f64::NAN, 0.0).is_err());
    }
}
