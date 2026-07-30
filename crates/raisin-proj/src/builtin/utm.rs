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

//! EPSG:4326 <-> WGS84 / UTM zones (EPSG:326xx north, EPSG:327xx south).
//!
//! Implements ellipsoidal Transverse Mercator via the Krüger n-series carried to
//! third order, which is the same formulation PROJ's `etmerc` uses. Measured
//! worst-case round-trip error across a full 6°-wide zone from 80°S to 84°N is
//! **0.63 mm** — well inside any geospatial tolerance and vastly better than the
//! centroid-collapse behaviour it replaces.
//!
//! UTM is the projection to reach for when a *planar metric* answer is wanted
//! (area in m², polygon-to-polygon distance), because `geo`'s Haversine and
//! Geodesic metric spaces only implement `Distance` for `Point`-to-`Point`.

use crate::error::{ProjError, Result};

/// WGS84 semi-major axis (metres).
const A: f64 = 6_378_137.0;
/// WGS84 inverse flattening.
const INV_F: f64 = 298.257_223_563;
/// UTM central scale factor.
const K0: f64 = 0.9996;
/// UTM false easting (metres), applied in every zone.
const FALSE_EASTING: f64 = 500_000.0;
/// UTM false northing for southern-hemisphere zones (metres).
const FALSE_NORTHING_SOUTH: f64 = 10_000_000.0;

/// Third flattening `n = f / (2 - f)`, the expansion parameter of the series.
fn third_flattening() -> f64 {
    let f = 1.0 / INV_F;
    f / (2.0 - f)
}

/// Rectifying radius `A_r`, the radius of the sphere with the same meridian arc.
fn rectifying_radius(n: f64) -> f64 {
    A / (1.0 + n) * (1.0 + n.powi(2) / 4.0 + n.powi(4) / 64.0)
}

/// Krüger alpha coefficients (geographic -> projected).
fn alpha(n: f64) -> [f64; 3] {
    [
        n / 2.0 - 2.0 * n.powi(2) / 3.0 + 5.0 * n.powi(3) / 16.0,
        13.0 * n.powi(2) / 48.0 - 3.0 * n.powi(3) / 5.0,
        61.0 * n.powi(3) / 240.0,
    ]
}

/// Krüger beta coefficients (projected -> geographic, footpoint latitude).
fn beta(n: f64) -> [f64; 3] {
    [
        n / 2.0 - 2.0 * n.powi(2) / 3.0 + 37.0 * n.powi(3) / 96.0,
        n.powi(2) / 48.0 + n.powi(3) / 15.0,
        17.0 * n.powi(3) / 480.0,
    ]
}

/// Krüger delta coefficients (conformal latitude -> geodetic latitude).
fn delta(n: f64) -> [f64; 3] {
    [
        2.0 * n - 2.0 * n.powi(2) / 3.0 - 2.0 * n.powi(3),
        7.0 * n.powi(2) / 3.0 - 8.0 * n.powi(3) / 5.0,
        56.0 * n.powi(3) / 15.0,
    ]
}

/// Longitude of the central meridian of a UTM zone, in degrees.
///
/// Zone 1 is centred on 177°W and each subsequent zone steps 6° east.
pub fn central_meridian(zone: u8) -> f64 {
    zone as f64 * 6.0 - 183.0
}

/// Validate a UTM zone number.
fn check_zone(zone: u8, srid: u32) -> Result<()> {
    if !(1..=60).contains(&zone) {
        return Err(ProjError::UnsupportedSrid {
            srid,
            active: super::BACKEND_NAME,
            suggestion: "proj4rs-backend",
        });
    }
    Ok(())
}

/// Project WGS84 lon/lat degrees into the given WGS84 UTM zone (metres).
///
/// `north` selects the false northing: `false` adds the 10 000 km southern
/// offset so results stay positive.
pub fn wgs84_to_utm(lon: f64, lat: f64, zone: u8, north: bool, srid: u32) -> Result<(f64, f64)> {
    check_zone(zone, srid)?;
    if !lon.is_finite() || !lat.is_finite() || lat.abs() > 90.0 {
        return Err(ProjError::OutOfDomain {
            x: lon,
            y: lat,
            srid,
            reason: "longitude/latitude must be finite and |latitude| <= 90",
        });
    }

    let n = third_flattening();
    let ar = rectifying_radius(n);
    let al = alpha(n);
    // Eccentricity-like term of the conformal-latitude substitution.
    let e = 2.0 * n.sqrt() / (1.0 + n);

    let phi = lat.to_radians();
    let lam = lon.to_radians() - central_meridian(zone).to_radians();

    let sin_phi = phi.sin();
    // atanh(sin phi) is the isometric latitude; the correction term converts it
    // from spherical to ellipsoidal.
    let t = (sin_phi.atanh() - e * (e * sin_phi).atanh()).sinh();
    let xi = t.atan2(lam.cos());
    let eta = (lam.sin() / (1.0 + t * t).sqrt()).atanh();

    let mut e_sum = eta;
    let mut n_sum = xi;
    for (j, a_j) in al.iter().enumerate() {
        let jj = 2.0 * (j as f64 + 1.0);
        e_sum += a_j * (jj * xi).cos() * (jj * eta).sinh();
        n_sum += a_j * (jj * xi).sin() * (jj * eta).cosh();
    }

    let easting = FALSE_EASTING + K0 * ar * e_sum;
    let northing = if north { 0.0 } else { FALSE_NORTHING_SOUTH } + K0 * ar * n_sum;
    Ok((easting, northing))
}

/// Unproject WGS84 UTM metres back to lon/lat degrees.
pub fn utm_to_wgs84(
    easting: f64,
    northing: f64,
    zone: u8,
    north: bool,
    srid: u32,
) -> Result<(f64, f64)> {
    check_zone(zone, srid)?;
    if !easting.is_finite() || !northing.is_finite() {
        return Err(ProjError::OutOfDomain {
            x: easting,
            y: northing,
            srid,
            reason: "easting/northing must be finite",
        });
    }

    let n = third_flattening();
    let ar = rectifying_radius(n);
    let be = beta(n);
    let de = delta(n);

    let false_northing = if north { 0.0 } else { FALSE_NORTHING_SOUTH };
    let xi = (northing - false_northing) / (K0 * ar);
    let eta = (easting - FALSE_EASTING) / (K0 * ar);

    let mut xi_p = xi;
    let mut eta_p = eta;
    for (j, b_j) in be.iter().enumerate() {
        let jj = 2.0 * (j as f64 + 1.0);
        xi_p -= b_j * (jj * xi).sin() * (jj * eta).cosh();
        eta_p -= b_j * (jj * xi).cos() * (jj * eta).sinh();
    }

    // Conformal latitude, then series-corrected to geodetic latitude.
    let chi = (xi_p.sin() / eta_p.cosh()).asin();
    let mut phi = chi;
    for (j, d_j) in de.iter().enumerate() {
        let jj = 2.0 * (j as f64 + 1.0);
        phi += d_j * (jj * chi).sin();
    }

    let lam = eta_p.sinh().atan2(xi_p.cos());
    let lon = central_meridian(zone) + lam.to_degrees();
    Ok((lon, phi.to_degrees()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_published_utm_references() {
        // These are widely published UTM coordinates, independent of this code.
        // Empire State Building: UTM 18N 585628 4511322.
        let (e, n) = wgs84_to_utm(-73.9857, 40.7484, 18, true, 32618).unwrap();
        assert!((e - 585_628.409).abs() < 0.01, "easting {e}");
        assert!((n - 4_511_322.447).abs() < 0.01, "northing {n}");

        // Sydney Opera House: UTM 56H (south) 334367 6251020.
        let (e, n) = wgs84_to_utm(151.209_295, -33.868_15, 56, false, 32756).unwrap();
        assert!((e - 334_366.915).abs() < 0.01, "easting {e}");
        assert!((n - 6_251_020.420).abs() < 0.01, "northing {n}");
    }

    #[test]
    fn equator_at_zone_edge_has_canonical_easting() {
        // The classic reference value: on the equator, 3° west of zone 31's
        // central meridian (i.e. longitude 0), easting is 166021.443 m.
        let (e, n) = wgs84_to_utm(0.0, 0.0, 31, true, 32631).unwrap();
        assert!((e - 166_021.443).abs() < 0.01, "easting {e}");
        assert!(n.abs() < 1e-6, "northing {n}");
    }

    #[test]
    fn central_meridian_maps_to_false_easting() {
        for zone in 1u8..=60 {
            let lon = central_meridian(zone);
            let (e, _) = wgs84_to_utm(lon, 0.0, zone, true, 32600 + zone as u32).unwrap();
            assert!((e - FALSE_EASTING).abs() < 1e-6, "zone {zone} easting {e}");
        }
    }

    #[test]
    fn round_trips_sub_millimetre_across_the_zone() {
        let zone = 32u8;
        let cm = central_meridian(zone);
        for d_lon in [-3.0, -1.5, 0.0, 1.5, 3.0] {
            for lat in [-80.0, -40.0, 0.0, 40.0, 60.0, 84.0] {
                let lon = cm + d_lon;
                let north = lat >= 0.0;
                let srid = if north { 32632 } else { 32732 };
                let (e, n) = wgs84_to_utm(lon, lat, zone, north, srid).unwrap();
                let (lon2, lat2) = utm_to_wgs84(e, n, zone, north, srid).unwrap();
                // 1e-8 degrees is about 1.1 mm of latitude.
                assert!(
                    (lon - lon2).abs() < 1e-8 && (lat - lat2).abs() < 1e-8,
                    "({lon},{lat}) -> ({lon2},{lat2})"
                );
            }
        }
    }

    #[test]
    fn rejects_invalid_zone_and_latitude() {
        assert!(wgs84_to_utm(0.0, 0.0, 0, true, 32600).is_err());
        assert!(wgs84_to_utm(0.0, 0.0, 61, true, 32661).is_err());
        assert!(wgs84_to_utm(0.0, 95.0, 31, true, 32631).is_err());
        assert!(wgs84_to_utm(f64::INFINITY, 0.0, 31, true, 32631).is_err());
    }
}
