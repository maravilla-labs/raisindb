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

//! Reprojecting whole geometries, not just single coordinates.

use geo::MapCoords;
use geo_types::{Coord, Geometry};

use crate::crs::Crs;
use crate::error::Result;
use crate::transform_coord;

// `geo` re-exports `geo_types`, so depending on `geo` alone is enough.
use geo as geo_types;

/// Reproject every coordinate of a geometry from `from` to `to`.
///
/// Uses `geo`'s `MapCoords::try_map_coords`, so the first failing coordinate
/// aborts the whole geometry — a partially reprojected geometry would be
/// nonsense, and worse, silently plausible.
///
/// # Performance note for implementers
///
/// This resolves the CRS pair once per *coordinate*, which is fine for the
/// row-at-a-time `ST_TRANSFORM` evaluator but wasteful for bulk reprojection.
/// If a backfill job ever reprojects millions of geometries, hoist the backend
/// lookup out of the loop instead of calling this in a tight loop.
///
/// # Examples
///
/// ```
/// use geo::{Geometry, Point};
/// use raisin_proj::{Crs, transform_geometry};
///
/// let p = Geometry::Point(Point::new(8.54, 47.37));
/// let projected = transform_geometry(&p, Crs::WGS84, Crs::WEB_MERCATOR).unwrap();
/// match projected {
///     Geometry::Point(p) => assert!((p.x() - 950_668.45).abs() < 0.01),
///     _ => unreachable!(),
/// }
/// ```
pub fn transform_geometry(geom: &Geometry<f64>, from: Crs, to: Crs) -> Result<Geometry<f64>> {
    if from == to {
        return Ok(geom.clone());
    }
    geom.try_map_coords(|Coord { x, y }| {
        let (nx, ny) = transform_coord(from, to, x, y)?;
        Ok(Coord { x: nx, y: ny })
    })
}

/// Reproject a geometry into the WGS84 UTM zone that best fits it, returning the
/// projected geometry alongside the CRS chosen.
///
/// This is the bridge for *planar metric* work. `geo` only implements `Distance`
/// for `Point`-to-`Point` in the `Haversine` and `Geodesic` metric spaces, but
/// implements the full geometry matrix (including `Geometry`-to-`Geometry`) for
/// `Euclidean`. So the correct way to get a real polygon-to-polygon distance or
/// a true area is: project to UTM, then measure with `Euclidean`.
///
/// Returns `None` when the geometry has no coordinates to centre a zone on.
pub fn to_best_utm(geom: &Geometry<f64>, from: Crs) -> Result<Option<(Geometry<f64>, Crs)>> {
    let wgs = if from == Crs::WGS84 {
        geom.clone()
    } else {
        transform_geometry(geom, from, Crs::WGS84)?
    };

    let Some((lon, lat)) = representative_coord(&wgs) else {
        return Ok(None);
    };
    let utm = Crs::best_utm_for(lon, lat);
    Ok(Some((transform_geometry(&wgs, Crs::WGS84, utm)?, utm)))
}

/// Any one coordinate of the geometry, used only to pick a UTM zone.
fn representative_coord(geom: &Geometry<f64>) -> Option<(f64, f64)> {
    use geo::CoordsIter;
    geom.coords_iter().next().map(|c| (c.x, c.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{LineString, Point, Polygon};

    #[test]
    fn transforms_a_point() {
        let p = Geometry::Point(Point::new(8.54, 47.37));
        let out = transform_geometry(&p, Crs::WGS84, Crs::WEB_MERCATOR).unwrap();
        match out {
            Geometry::Point(p) => {
                assert!((p.x() - 950_668.451_374_556_3).abs() < 1e-3);
                assert!((p.y() - 6_002_677.997_532_714).abs() < 1e-3);
            }
            other => panic!("expected Point, got {other:?}"),
        }
    }

    #[test]
    fn identity_clones_without_touching_coords() {
        let p = Geometry::Point(Point::new(1.25, 2.5));
        let out = transform_geometry(&p, Crs::WGS84, Crs::WGS84).unwrap();
        assert_eq!(out, p);
    }

    #[test]
    fn round_trips_a_polygon() {
        let poly = Geometry::Polygon(Polygon::new(
            LineString::from(vec![
                (8.50, 47.35),
                (8.60, 47.35),
                (8.60, 47.40),
                (8.50, 47.40),
                (8.50, 47.35),
            ]),
            vec![],
        ));
        let projected = transform_geometry(&poly, Crs::WGS84, Crs::WEB_MERCATOR).unwrap();
        let back = transform_geometry(&projected, Crs::WEB_MERCATOR, Crs::WGS84).unwrap();
        let (a, b) = match (&poly, &back) {
            (Geometry::Polygon(a), Geometry::Polygon(b)) => (a, b),
            _ => unreachable!(),
        };
        for (ca, cb) in a.exterior().coords().zip(b.exterior().coords()) {
            assert!((ca.x - cb.x).abs() < 1e-9, "{ca:?} vs {cb:?}");
            assert!((ca.y - cb.y).abs() < 1e-9, "{ca:?} vs {cb:?}");
        }
    }

    #[test]
    fn a_failing_coordinate_aborts_the_whole_geometry() {
        // One pole among otherwise fine coordinates must fail the geometry, not
        // silently produce a partially projected ring.
        let ls = Geometry::LineString(LineString::from(vec![(0.0, 0.0), (0.0, 90.0)]));
        assert!(transform_geometry(&ls, Crs::WGS84, Crs::WEB_MERCATOR).is_err());
    }

    #[test]
    fn best_utm_gives_metric_coordinates_and_the_zone() {
        let poly = Geometry::Polygon(Polygon::new(
            LineString::from(vec![
                (8.50, 47.35),
                (8.60, 47.35),
                (8.60, 47.40),
                (8.50, 47.40),
                (8.50, 47.35),
            ]),
            vec![],
        ));
        let (utm_geom, crs) = to_best_utm(&poly, Crs::WGS84).unwrap().unwrap();
        assert_eq!(crs, Crs::from_srid(32632));

        // The whole point of projecting: Euclidean area is now real square metres.
        use geo::Area;
        let area = utm_geom.unsigned_area();
        // ~7.5 km x ~5.6 km box over Zurich => on the order of 4.2e7 m².
        assert!(
            (3.5e7..5.0e7).contains(&area),
            "unexpected projected area {area}"
        );
    }
}
