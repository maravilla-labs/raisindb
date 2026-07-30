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

//! Measurement, with one rule stated once.
//!
//! **Topological predicates and set operations are planar in the geometry's own
//! coordinate space; measurements are geodesic when the CRS is geographic and
//! planar when it is projected.**
//!
//! On EPSG:4326 that makes `ST_AREA` square metres and `ST_LENGTH` metres, which
//! diverges from PostGIS's `geometry` type (square degrees, degrees) and matches
//! its `geography` type. RaisinDB has one geometry type and selects the semantics
//! from the SRID, so the useful answer is the only one on offer. The divergence is
//! deliberate and documented rather than silent.
//!
//! # Why a projection is unavoidable for distance
//!
//! `geo`'s `HaversineMeasure` and `GeodesicMeasure` implement `Distance` for
//! **Point-to-Point only**; only `Euclidean` has the full geometry-to-geometry
//! matrix. So the old centroid-to-centroid fallback for polygon-to-polygon was
//! *not* fixable by swapping a trait — it required projecting into a metric CRS
//! first. That is what [`distance`] does, and it is why `raisin-proj` is a hard
//! dependency of a correct `ST_DISTANCE` and not only of `ST_TRANSFORM`.

use geo::{
    Area, Bearing, Distance, Euclidean, GeodesicArea, Geometry, Haversine, Length, LineString,
    Polygon,
};
use raisin_error::Error;
use raisin_geometry::{Crs, Geom};

use super::walk::{for_each_line_string, for_each_polygon, for_each_ring, single_point};

/// Area in square metres on a geographic CRS, square native units on a projected
/// one.
///
/// Uses Karney's 2013 ellipsoidal formula (`GeodesicArea`) rather than the
/// spherical Chamberlain-Duquette one the previous implementation used: same
/// call cost, better numbers. Linear and puntal components contribute 0, as they
/// must, and `Multi*` / nested `GeometryCollection`s sum their areal members.
///
/// # Why this does not call `Geometry::geodesic_area_unsigned` directly
///
/// Because that answer depends on **ring winding**, and for the wrong winding it
/// is not slightly off — it is the surface area of the Earth. `geo`'s
/// `geodesic_area` declares `Winding::CounterClockwise` for the exterior ring and
/// `Winding::Clockwise` for interiors, and `geographiclib`'s unsigned `compute`
/// returns `earth_area - |A|` for a ring wound against its declaration. Neither
/// the `.abs()` on the interior sum nor the sign fix-up afterwards recovers from
/// that. Measured through SQL before this was corrected:
///
/// | input | returned | truth |
/// |---|---|---|
/// | CCW 1°×1° square at the equator | 1.23e10 | 1.23e10 |
/// | the **same square wound CW** | 5.10e14 | 1.23e10 |
/// | CCW square with a CW hole | 9.23e9 | 9.23e9 |
/// | CCW square with a **CCW hole** | -5.10e14 | 9.23e9 |
///
/// Clockwise exterior rings are not exotic: OGC shapefiles wind exterior rings
/// clockwise, so every polygon imported from a shapefile hit the second row. RFC
/// 7946 §3.1.6 recommends the right-hand rule but explicitly requires parsers not
/// to reject other winding, and PostGIS's `ST_Area` is winding-independent. So
/// area is accumulated per ring instead, taking `|signed|` of each: the *signed*
/// geodesic area has the correct magnitude under either winding, which is what
/// makes the result depend on the shape rather than on how it was written down.
pub(super) fn area(g: &Geom) -> f64 {
    if !g.is_geographic() {
        // `geo`'s planar `Area for Polygon` already takes `.abs()` per ring, so
        // it is winding-independent and needs no help.
        return g.geometry.unsigned_area();
    }
    let mut total = 0.0;
    for_each_polygon(&g.geometry, &mut |p| {
        let mut poly = ring_area(p.exterior());
        for hole in p.interiors() {
            poly -= ring_area(hole);
        }
        // A hole larger than its shell is malformed input, not negative area.
        total += poly.max(0.0);
    });
    total
}

/// Ellipsoidal area enclosed by one ring, independent of its winding.
fn ring_area(ring: &LineString<f64>) -> f64 {
    Polygon::new(ring.clone(), Vec::new())
        .geodesic_area_signed()
        .abs()
}

/// Length of the 1-dimensional components only.
///
/// Areal components contribute 0 — that is PostGIS's `ST_Length`, and
/// [`perimeter`] is the function for a polygon's boundary. A
/// `GeometryCollection`'s linear parts are summed.
pub(super) fn length(g: &Geom) -> f64 {
    let mut total = 0.0;
    let geographic = g.is_geographic();
    for_each_line_string(&g.geometry, &mut |ls| {
        total += if geographic {
            Haversine.length(ls)
        } else {
            Euclidean.length(ls)
        };
    });
    total
}

/// Boundary length of the 2-dimensional components only, interior rings included.
///
/// Puntal and linear components contribute 0, matching PostGIS's `ST_Perimeter`.
pub(super) fn perimeter(g: &Geom) -> f64 {
    let mut total = 0.0;
    let geographic = g.is_geographic();
    for_each_ring(&g.geometry, &mut |ring| {
        total += if geographic {
            Haversine.length(ring)
        } else {
            Euclidean.length(ring)
        };
    });
    total
}

/// Minimum distance between two geometries: metres on a geographic CRS, native
/// units on a projected one.
///
/// Three paths, in order of fidelity:
///
/// 1. **Projected CRS** — `Euclidean.distance` over `&Geometry`, which `geo`
///    implements for every type pair. Exact for the coordinate space given.
/// 2. **Geographic, both operands a single point** — `Haversine.distance`. Exact
///    geodesic, and this is the overwhelmingly common case (`ST_DISTANCE` between
///    two locations).
/// 3. **Geographic, anything else** — project *both* operands into one shared UTM
///    zone and measure with `Euclidean`. This is true minimum shape-to-shape
///    distance, replacing the centroid-to-centroid approximation.
///
/// The accuracy note for path 3, stated rather than hidden: UTM is a conformal
/// projection with scale error around 0.1% at the edge of a zone, so a distance
/// between operands far from the chosen zone's central meridian is approximate.
/// Both operands are deliberately projected with the **same** zone — projecting
/// each into its own best-fitting zone would place them in different coordinate
/// spaces and the subtraction would be meaningless.
pub(super) fn distance(a: &Geom, b: &Geom) -> Result<f64, Error> {
    raisin_geometry::require_same_srid("ST_DISTANCE", a, b)?;

    if !a.is_geographic() {
        return Ok(Euclidean.distance(&a.geometry, &b.geometry));
    }

    if let (Some(pa), Some(pb)) = (single_point(&a.geometry), single_point(&b.geometry)) {
        return Ok(Haversine.distance(pa, pb));
    }

    let (ga, gb) = shared_metric(a, b)?;
    Ok(Euclidean.distance(&ga, &gb))
}

/// Geodesic bearing from `a` to `b` in radians, north-clockwise, normalized to
/// `[0, 2pi)`.
///
/// `None` when either side is not a single location or the two coincide — the
/// azimuth between a point and itself is undefined, and PostGIS returns NULL
/// there rather than an arbitrary 0.
pub(super) fn bearing_radians(a: &Geom, b: &Geom) -> Option<f64> {
    let (pa, pb) = (single_point(&a.geometry)?, single_point(&b.geometry)?);
    if pa == pb {
        return None;
    }
    let degrees = if a.is_geographic() {
        Haversine.bearing(pa, pb)
    } else {
        Euclidean.bearing(pa, pb)
    };
    let two_pi = std::f64::consts::TAU;
    Some((degrees.to_radians().rem_euclid(two_pi) + two_pi) % two_pi)
}

/// Project both operands into a single shared metric CRS.
///
/// The zone is chosen from the midpoint of the two bounding boxes so that neither
/// operand is systematically favoured, and both are then transformed with that one
/// zone.
pub(super) fn shared_metric(a: &Geom, b: &Geom) -> Result<(Geometry<f64>, Geometry<f64>), Error> {
    let wgs_a = raisin_geometry::transform(a, Crs::WGS84)?;
    let wgs_b = raisin_geometry::transform(b, Crs::WGS84)?;

    let Some((lon, lat)) = midpoint(&wgs_a, &wgs_b) else {
        // At least one side is empty; there is nothing to measure and no zone to
        // centre. Hand back the unprojected geometries — `Euclidean.distance`
        // over an empty geometry is the caller's problem to interpret, and it
        // cannot be wrong because there are no coordinates.
        return Ok((wgs_a.geometry, wgs_b.geometry));
    };

    let zone = Crs::best_utm_for(lon, lat);
    Ok((
        raisin_geometry::transform(&wgs_a, zone)?.geometry,
        raisin_geometry::transform(&wgs_b, zone)?.geometry,
    ))
}

/// Centre of the combined bounding box of two geometries, in lon/lat.
fn midpoint(a: &Geom, b: &Geom) -> Option<(f64, f64)> {
    let (ea, eb) = (a.envelope(), b.envelope());
    let e = match (ea, eb) {
        (Some(x), Some(y)) => raisin_geometry::Envelope {
            min_x: x.min_x.min(y.min_x),
            min_y: x.min_y.min(y.min_y),
            max_x: x.max_x.max(y.max_x),
            max_y: x.max_y.max(y.max_y),
        },
        (Some(x), None) | (None, Some(x)) => x,
        (None, None) => return None,
    };
    Some(((e.min_x + e.max_x) / 2.0, (e.min_y + e.max_y) / 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{LineString, MultiPolygon, Point, Polygon};

    fn wgs(g: Geometry<f64>) -> Geom {
        Geom::wgs84(g)
    }

    fn square(x0: f64, y0: f64, side: f64) -> Polygon<f64> {
        Polygon::new(
            LineString::from(vec![
                (x0, y0),
                (x0 + side, y0),
                (x0 + side, y0 + side),
                (x0, y0 + side),
                (x0, y0),
            ]),
            vec![],
        )
    }

    /// The headline divergence from PostGIS's `geometry` type: a 4326 area is
    /// square METRES, not square degrees.
    #[test]
    fn area_on_lon_lat_is_square_metres() {
        // ~0.01 degree square over Zurich: roughly 750m x 1100m.
        let g = wgs(Geometry::Polygon(square(8.5, 47.35, 0.01)));
        let a = area(&g);
        assert!(
            (6.0e5..1.2e6).contains(&a),
            "expected ~8.3e5 m^2, got {a} (square degrees would be 1e-4)"
        );
    }

    #[test]
    fn area_on_a_projected_crs_is_native_units_squared() {
        let g = Geom::new(
            Geometry::Polygon(square(500_000.0, 5_000_000.0, 100.0)),
            Crs::from_srid(32632),
        );
        assert!((area(&g) - 10_000.0).abs() < 1e-6, "{}", area(&g));
    }

    /// `Multi*` and `GeometryCollection` as INPUT — the gap that made
    /// `ST_AREA(ST_UNION(a, b))` fail whenever the union yielded a MultiPolygon.
    #[test]
    fn area_sums_over_multipolygon_and_collections() {
        let two = MultiPolygon(vec![square(8.5, 47.35, 0.01), square(9.5, 47.35, 0.01)]);
        let multi = area(&wgs(Geometry::MultiPolygon(two.clone())));
        let single = area(&wgs(Geometry::Polygon(two.0[0].clone())));
        assert!(
            (multi / single - 2.0).abs() < 0.05,
            "two equal squares should be ~2x one: {multi} vs {single}"
        );

        let collection = Geometry::GeometryCollection(
            vec![
                Geometry::MultiPolygon(two),
                Geometry::Point(Point::new(0.0, 0.0)),
            ]
            .into(),
        );
        assert!(
            (area(&wgs(collection)) - multi).abs() < 1.0,
            "a point adds 0"
        );
    }

    /// Interior rings must be SUBTRACTED, not ignored.
    #[test]
    fn area_subtracts_holes() {
        let with_hole = Polygon::new(
            square(0.0, 0.0, 1.0).exterior().clone(),
            vec![square(0.25, 0.25, 0.5).exterior().clone()],
        );
        let solid = area(&wgs(Geometry::Polygon(square(0.0, 0.0, 1.0))));
        let holed = area(&wgs(Geometry::Polygon(with_hole)));
        assert!(holed < solid * 0.8, "{holed} vs {solid}");
    }

    #[test]
    fn length_is_metres_and_ignores_areal_components() {
        // One degree of latitude is ~111 km.
        let line = Geometry::LineString(LineString::from(vec![(0.0, 0.0), (0.0, 1.0)]));
        let l = length(&wgs(line));
        assert!((1.10e5..1.12e5).contains(&l), "{l}");

        assert_eq!(
            length(&wgs(Geometry::Polygon(square(0.0, 0.0, 1.0)))),
            0.0,
            "ST_LENGTH of an areal geometry is 0; ST_PERIMETER is the one that measures its boundary"
        );
        assert_eq!(length(&wgs(Geometry::Point(Point::new(1.0, 2.0)))), 0.0);
    }

    #[test]
    fn perimeter_counts_every_ring_and_only_areal_components() {
        let with_hole = Polygon::new(
            square(0.0, 0.0, 1.0).exterior().clone(),
            vec![square(0.25, 0.25, 0.5).exterior().clone()],
        );
        let solid = perimeter(&wgs(Geometry::Polygon(square(0.0, 0.0, 1.0))));
        let holed = perimeter(&wgs(Geometry::Polygon(with_hole)));
        assert!(
            holed > solid,
            "an interior ring adds boundary: {holed} vs {solid}"
        );

        assert_eq!(
            perimeter(&wgs(Geometry::LineString(LineString::from(vec![
                (0.0, 0.0),
                (1.0, 1.0)
            ])))),
            0.0
        );
    }

    #[test]
    fn point_to_point_distance_is_exact_geodesic() {
        // One degree of latitude at the equator.
        let a = wgs(Geometry::Point(Point::new(0.0, 0.0)));
        let b = wgs(Geometry::Point(Point::new(0.0, 1.0)));
        let d = distance(&a, &b).unwrap();
        assert!((1.10e5..1.12e5).contains(&d), "{d}");
    }

    /// The defect this module exists to fix: polygon-to-polygon was
    /// centroid-to-centroid, which is non-zero for overlapping shapes and far too
    /// large for adjacent ones.
    #[test]
    fn polygon_to_polygon_is_true_minimum_not_centroid_to_centroid() {
        let left = wgs(Geometry::Polygon(square(8.50, 47.35, 0.01)));
        let right = wgs(Geometry::Polygon(square(8.52, 47.35, 0.01)));

        let d = distance(&left, &right).unwrap();
        // Gap is 0.01 degrees of longitude at 47.35N ~= 754 m. Centroid to
        // centroid would be ~1500 m.
        assert!((600.0..900.0).contains(&d), "expected ~754 m gap, got {d}");

        let overlapping = wgs(Geometry::Polygon(square(8.505, 47.35, 0.01)));
        assert_eq!(
            distance(&left, &overlapping).unwrap(),
            0.0,
            "overlapping shapes are zero apart; the centroid fallback returned a positive number"
        );
    }

    #[test]
    fn point_inside_a_polygon_is_zero_away_from_it() {
        let poly = wgs(Geometry::Polygon(square(8.5, 47.35, 0.02)));
        let inside = wgs(Geometry::Point(Point::new(8.51, 47.36)));
        assert_eq!(distance(&poly, &inside).unwrap(), 0.0);
    }

    /// Both operands must land in the SAME zone, or the subtraction is nonsense.
    #[test]
    fn shared_metric_uses_one_zone_for_both_operands() {
        // Zurich (zone 32) and Vienna (zone 33): a naive per-geometry
        // `to_best_utm` would put these in different coordinate spaces and report
        // a distance of a few kilometres instead of a few hundred.
        let zurich = wgs(Geometry::Polygon(square(8.5, 47.35, 0.01)));
        let vienna = wgs(Geometry::Polygon(square(16.35, 48.2, 0.01)));
        let d = distance(&zurich, &vienna).unwrap();
        assert!(
            (5.5e5..6.5e5).contains(&d),
            "Zurich to Vienna is ~600 km, got {d}"
        );
    }

    #[test]
    fn distance_on_a_projected_crs_is_planar_native_units() {
        let utm = Crs::from_srid(32632);
        let a = Geom::new(Geometry::Point(Point::new(500_000.0, 5_000_000.0)), utm);
        let b = Geom::new(Geometry::Point(Point::new(500_300.0, 5_000_400.0)), utm);
        assert!((distance(&a, &b).unwrap() - 500.0).abs() < 1e-6);
    }

    #[test]
    fn azimuth_is_north_clockwise_radians_and_undefined_for_coincident_points() {
        let origin = wgs(Geometry::Point(Point::new(0.0, 0.0)));
        let north = wgs(Geometry::Point(Point::new(0.0, 1.0)));
        let east = wgs(Geometry::Point(Point::new(1.0, 0.0)));

        assert!(bearing_radians(&origin, &north).unwrap().abs() < 1e-9);
        let e = bearing_radians(&origin, &east).unwrap();
        assert!(
            (e - std::f64::consts::FRAC_PI_2).abs() < 1e-6,
            "east is pi/2, got {e}"
        );

        assert!(
            bearing_radians(&origin, &origin).is_none(),
            "the azimuth from a point to itself is undefined, not 0"
        );
        assert!(bearing_radians(&origin, &wgs(Geometry::Polygon(square(0.0, 0.0, 1.0)))).is_none());
    }

    #[test]
    fn measurements_over_an_empty_geometry_are_zero_rather_than_errors() {
        let e = wgs(Geometry::GeometryCollection(Default::default()));
        assert_eq!(area(&e), 0.0);
        assert_eq!(length(&e), 0.0);
        assert_eq!(perimeter(&e), 0.0);
    }
}
