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

//! Projecting into a metric CRS so planar `geo` algorithms mean metres.
//!
//! # Why this is not optional
//!
//! `geo`'s `HaversineMeasure` and `GeodesicMeasure` implement `Distance` for
//! **Point-to-Point only**; only `Euclidean` has the full geometry-to-geometry
//! matrix. So a correct polygon-to-polygon distance is *not* obtainable by
//! swapping a trait — the old centroid-to-centroid fallback cannot be fixed that
//! way. The only correct route is: project both operands to a metric CRS, then
//! measure with `Euclidean`.
//!
//! The same applies to `Buffer` and `Simplify`, which are planar and work in the
//! geometry's own units. On EPSG:4326 `buffer(50)` means **50 degrees**. Swapping
//! the old centroid-and-32-gon `ST_BUFFER` for a bare `.buffer()` would trade one
//! wrong answer for another; the shape is
//! [`to_metric`] -> operate -> [`from_metric`].

use geo::Geometry;
use raisin_proj::Crs;

use crate::error::Result;
use crate::geom::Geom;

/// Project a geometry into a CRS whose units are metres, returning the CRS so the
/// caller can project a result back.
///
/// * Already in a WGS84 UTM zone → returned unchanged (its units are already
///   metres and its own zone is the best fit for it).
/// * Anything else → reprojected into the best-fitting WGS84 UTM zone.
/// * Empty → returned unchanged with its own CRS. There are no coordinates, so no
///   measurement over it can be wrong, and this saves every caller a special case.
///
/// EPSG:3857 is deliberately *not* treated as metric: its metres are
/// Mercator-distorted by roughly `1/cos(latitude)` — about 1.5x at 48°N — so a
/// 3857 length is not a ground distance.
pub fn to_metric(g: &Geom) -> Result<(Geometry<f64>, Crs)> {
    if g.srid.utm_zone().is_some() || g.is_empty() {
        return Ok((g.geometry.clone(), g.srid));
    }
    match raisin_proj::geometry::to_best_utm(&g.geometry, g.srid)? {
        Some((geometry, crs)) => Ok((geometry, crs)),
        // `to_best_utm` returns None only when there is no coordinate to centre a
        // zone on, which `is_empty` above already caught; keep the arm total.
        None => Ok((g.geometry.clone(), g.srid)),
    }
}

/// Bring a geometry produced in a metric CRS back into `to`.
///
/// The vertical extent is not carried through (a projection has nothing to say
/// about altitude); re-attach it with
/// [`Geom::with_z_range`](crate::Geom::with_z_range) when it matters.
pub fn from_metric(g: Geometry<f64>, from: Crs, to: Crs) -> Result<Geom> {
    Ok(Geom::new(
        raisin_proj::transform_geometry(&g, from, to)?,
        to,
    ))
}

/// Reproject a whole [`Geom`] into `to`, preserving the vertical extent.
///
/// This is what `ST_TRANSFORM` is built on. All-or-nothing: one bad coordinate
/// fails the whole geometry rather than emitting a half-projected ring.
pub fn transform(g: &Geom, to: Crs) -> Result<Geom> {
    if g.srid == to {
        return Ok(g.clone());
    }
    Ok(Geom::new(
        raisin_proj::transform_geometry(&g.geometry, g.srid, to)?,
        to,
    )
    .with_z_range(g.z_range))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Area, LineString, Point, Polygon};

    fn zurich_box() -> Geom {
        Geom::wgs84(Geometry::Polygon(Polygon::new(
            LineString::from(vec![
                (8.50, 47.35),
                (8.60, 47.35),
                (8.60, 47.40),
                (8.50, 47.40),
                (8.50, 47.35),
            ]),
            vec![],
        )))
    }

    /// The whole point: after projection, a planar `geo` area is real m².
    #[test]
    fn to_metric_makes_planar_area_mean_square_metres() {
        let (geometry, crs) = to_metric(&zurich_box()).unwrap();
        assert_eq!(crs, Crs::from_srid(32632), "UTM zone 32 north");
        let area = geometry.unsigned_area();
        assert!(
            (3.5e7..5.0e7).contains(&area),
            "~7.5km x ~5.6km over Zurich should be ~4.2e7 m², got {area}"
        );

        // In degrees the same polygon's "area" is 0.005 — the number the old
        // planar-on-4326 path would have produced.
        let degrees = zurich_box().geometry.unsigned_area();
        assert!(degrees < 1.0, "{degrees}");
    }

    #[test]
    fn a_geometry_already_in_utm_is_not_reprojected() {
        let (utm_geom, crs) = to_metric(&zurich_box()).unwrap();
        let already = Geom::new(utm_geom.clone(), crs);
        let (again, again_crs) = to_metric(&already).unwrap();
        assert_eq!(again_crs, crs);
        assert_eq!(again, utm_geom, "must be a no-op, not a round trip");
    }

    #[test]
    fn web_mercator_is_reprojected_because_its_metres_are_distorted() {
        let mercator = transform(&zurich_box(), Crs::WEB_MERCATOR).unwrap();
        let distorted = mercator.geometry.unsigned_area();
        let (metric, crs) = to_metric(&mercator).unwrap();
        assert_eq!(crs, Crs::from_srid(32632));
        let real = metric.unsigned_area();
        // At ~47°N the Mercator exaggeration is ~1/cos(47)² ≈ 2.1x in area.
        assert!(
            distorted > real * 1.8,
            "expected Mercator area {distorted} to exceed true area {real}"
        );
    }

    #[test]
    fn an_empty_geometry_needs_no_zone_and_does_not_error() {
        let empty = Geom::wgs84(Geometry::GeometryCollection(Default::default()));
        let (geometry, crs) = to_metric(&empty).unwrap();
        assert_eq!(crs, Crs::WGS84);
        assert_eq!(geometry, empty.geometry);
    }

    #[test]
    fn metric_round_trip_returns_to_the_original_coordinates() {
        let original = zurich_box();
        let (metric, crs) = to_metric(&original).unwrap();
        let back = from_metric(metric, crs, Crs::WGS84).unwrap();
        let (a, b) = match (&original.geometry, &back.geometry) {
            (Geometry::Polygon(a), Geometry::Polygon(b)) => (a, b),
            _ => unreachable!(),
        };
        for (ca, cb) in a.exterior().coords().zip(b.exterior().coords()) {
            assert!((ca.x - cb.x).abs() < 1e-8, "{ca:?} vs {cb:?}");
            assert!((ca.y - cb.y).abs() < 1e-8, "{ca:?} vs {cb:?}");
        }
    }

    #[test]
    fn transform_preserves_the_vertical_extent_and_short_circuits_identity() {
        let g = Geom::wgs84(Geometry::Point(Point::new(8.54, 47.37)))
            .with_z_range(Some((400.0, 420.0)));
        let out = transform(&g, Crs::WEB_MERCATOR).unwrap();
        assert_eq!(out.z_range, Some((400.0, 420.0)));
        assert_eq!(out.srid, Crs::WEB_MERCATOR);

        assert_eq!(transform(&g, Crs::WGS84).unwrap(), g);
    }

    /// The error must name the Cargo feature, verbatim from `ProjError` — there
    /// is deliberately no silent-passthrough path.
    #[test]
    fn an_unsupported_target_fails_loudly_and_actionably() {
        let err = transform(&zurich_box(), Crs::from_srid(999_999)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("999999"), "{msg}");
        assert!(msg.contains("features") || msg.contains("backend"), "{msg}");
    }
}
