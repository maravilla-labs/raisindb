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

//! `ST_BUFFER` and `ST_SIMPLIFY`: planar `geo` algorithms whose *argument* is a
//! distance, run so that the distance means metres.
//!
//! # The trap this module exists to avoid
//!
//! `geo::Buffer` and `geo::Simplify` are planar and work in the geometry's **own
//! coordinate units**. On EPSG:4326 that makes `buffer(50)` fifty *degrees* —
//! about 5,500 km. So swapping the old centroid-and-32-gon `ST_BUFFER` for a bare
//! `.buffer()` would trade one wrong answer for another. The correct shape is
//! project into a metric CRS, operate, project back:
//!
//! ```text
//! to_metric -> buffer(metres) -> from_metric
//! ```
//!
//! On a projected CRS the coordinates are already linear, so the operation runs
//! directly and the distance is in that CRS's native units. That is the same
//! geodesic-versus-planar rule the measurement functions follow, applied to an
//! input argument rather than an output value.

use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::{Buffer, Geometry, MultiPolygon, Simplify};
use raisin_error::Error;
use raisin_geometry::Geom;

use super::convert::narrow_multipolygon;

/// Buffer a geometry by `distance`, in metres on a geographic CRS and in native
/// units on a projected one.
///
/// `quad_segments`, when given, is the number of segments used per quarter circle
/// — the `ST_BUFFER(geometry, distance, num_seg_quarter_circle)` overload. `geo`
/// expresses this through `LineJoin::Round`'s approximation tolerance, so the
/// value is converted into the tolerance that yields that many segments.
///
/// A negative distance erodes a polygon, as in PostGIS, and can legitimately
/// produce an empty result. Emptiness propagates rather than erroring.
pub(super) fn buffer(g: &Geom, distance: f64, quad_segments: Option<i64>) -> Result<Geom, Error> {
    if !distance.is_finite() {
        return Err(Error::Validation(
            "ST_BUFFER: distance must be finite".to_string(),
        ));
    }
    if g.is_empty() {
        return Ok(g.clone());
    }

    let style = quad_segments
        .map(|segments| style_for(distance, segments))
        .transpose()?;

    if !g.is_geographic() {
        let buffered = run_buffer(&g.geometry, distance, style);
        return Ok(g.map_geometry(narrow_multipolygon(buffered)));
    }

    let (metric, metric_crs) = raisin_geometry::to_metric(g)?;
    let buffered = run_buffer(&metric, distance, style);
    let back = raisin_geometry::from_metric(narrow_multipolygon(buffered), metric_crs, g.srid)?;
    Ok(back.with_z_range(g.z_range))
}

fn run_buffer(
    g: &Geometry<f64>,
    distance: f64,
    style: Option<BufferStyle<f64>>,
) -> MultiPolygon<f64> {
    match style {
        Some(style) => g.buffer_with_style(style),
        None => g.buffer(distance),
    }
}

/// Translate a PostGIS quarter-circle segment count into `geo`'s round-arc
/// parameter.
///
/// `LineJoin::Round(x)` and `LineCap::Round(x)` take `x = L / R`, the ratio of the
/// **maximum segment length to the arc radius** — not an absolute tolerance. A
/// quarter circle (pi/2 radians) split into `n` equal chords gives each chord a
/// length of `2R sin(pi / 4n)`, so the ratio is `2 sin(pi / 4n)`. Because it is a
/// ratio it is scale-free, which is why the result is independent of the buffer
/// distance and of the CRS.
///
/// Both the join and the cap are set: leaving the cap at `geo`'s default would make
/// a line's rounded ends finer than its corners, so the requested segment count
/// would only half apply.
fn style_for(distance: f64, quad_segments: i64) -> Result<BufferStyle<f64>, Error> {
    if quad_segments < 1 {
        return Err(Error::Validation(
            "ST_BUFFER: the segment count per quarter circle must be at least 1".to_string(),
        ));
    }
    let ratio = 2.0 * (std::f64::consts::FRAC_PI_4 / quad_segments as f64).sin();
    Ok(BufferStyle::new(distance)
        .line_join(LineJoin::Round(ratio))
        .line_cap(LineCap::Round(ratio)))
}

/// Simplify a geometry with Ramer-Douglas-Peucker, `tolerance` in metres on a
/// geographic CRS and native units on a projected one.
///
/// Puntal components pass through unchanged (there is nothing to remove) and
/// areal components keep their rings closed, both of which `geo`'s per-type
/// `Simplify` impls guarantee. `geo` has no impl for `Geometry` itself, hence the
/// dispatch here.
pub(super) fn simplify(g: &Geom, tolerance: f64) -> Result<Geom, Error> {
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(Error::Validation(
            "ST_SIMPLIFY: tolerance must be a non-negative finite number".to_string(),
        ));
    }
    // Short-circuit when there is nothing to remove. Without this, simplifying a
    // Point on a geographic CRS would project it to UTM and back for no reason,
    // and the round trip perturbs the coordinate in the last few decimal places —
    // so a no-op would silently move the geometry.
    if g.is_empty() || !has_simplifiable_component(&g.geometry) {
        return Ok(g.clone());
    }

    if !g.is_geographic() {
        return Ok(g.map_geometry(simplify_geometry(&g.geometry, tolerance)));
    }

    let (metric, metric_crs) = raisin_geometry::to_metric(g)?;
    let simplified = simplify_geometry(&metric, tolerance);
    let back = raisin_geometry::from_metric(simplified, metric_crs, g.srid)?;
    Ok(back.with_z_range(g.z_range))
}

/// True when the geometry has a component with vertices that could be dropped.
///
/// Puntal components never do, so a Point or MultiPoint is left exactly as it
/// arrived.
fn has_simplifiable_component(g: &Geometry<f64>) -> bool {
    let mut simplifiable = false;
    super::walk::for_each_line_string(g, &mut |_| simplifiable = true);
    super::walk::for_each_polygon(g, &mut |_| simplifiable = true);
    simplifiable
}

fn simplify_geometry(g: &Geometry<f64>, tolerance: f64) -> Geometry<f64> {
    match g {
        Geometry::LineString(ls) => Geometry::LineString(ls.simplify(tolerance)),
        Geometry::MultiLineString(mls) => Geometry::MultiLineString(mls.simplify(tolerance)),
        Geometry::Polygon(p) => Geometry::Polygon(p.simplify(tolerance)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.simplify(tolerance)),
        // Two points have nothing to drop between them.
        Geometry::Line(l) => Geometry::LineString(geo::LineString::from(vec![l.start, l.end])),
        Geometry::Rect(r) => Geometry::Polygon(r.to_polygon().simplify(tolerance)),
        Geometry::Triangle(t) => Geometry::Polygon(t.to_polygon().simplify(tolerance)),
        Geometry::GeometryCollection(gc) => Geometry::GeometryCollection(
            gc.0.iter()
                .map(|m| simplify_geometry(m, tolerance))
                .collect::<Vec<_>>()
                .into(),
        ),
        // Nothing to simplify about a set of locations.
        Geometry::Point(_) | Geometry::MultiPoint(_) => g.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Area, Distance, Haversine, LineString, MultiLineString, Point, Polygon};
    use raisin_geometry::Crs;

    fn wgs(g: Geometry<f64>) -> Geom {
        Geom::wgs84(g)
    }

    /// The whole point: a 1000 m buffer must be 1000 m, not 1000 degrees.
    #[test]
    fn buffering_a_point_on_lon_lat_uses_metres() {
        let zurich = wgs(Geometry::Point(Point::new(8.54, 47.37)));
        let disc = buffer(&zurich, 1000.0, None).unwrap();

        let ring = match &disc.geometry {
            Geometry::Polygon(p) => p.exterior().clone(),
            other => panic!("expected a Polygon, got {other:?}"),
        };
        let centre = Point::new(8.54, 47.37);
        for c in ring.coords() {
            let r = Haversine.distance(centre, Point::new(c.x, c.y));
            assert!(
                (960.0..1040.0).contains(&r),
                "every vertex should sit ~1000 m out, got {r} m"
            );
        }
    }

    /// The defect: every non-Point collapsed to its centroid, so a long road's
    /// buffer was a disc at its middle instead of a corridor along it.
    #[test]
    fn buffering_a_line_follows_the_line_rather_than_its_centroid() {
        let road = wgs(Geometry::LineString(LineString::from(vec![
            (8.50, 47.37),
            (8.60, 47.37),
        ])));
        let corridor = buffer(&road, 200.0, None).unwrap();
        let e = corridor.envelope().unwrap();

        // ~7.5 km long, ~400 m wide: the box must be far wider than it is tall.
        let width = e.max_x - e.min_x;
        let height = e.max_y - e.min_y;
        assert!(
            width > height * 5.0,
            "a corridor, not a disc: {width} x {height}"
        );
        assert!(width > 0.10, "must span the whole road: {width}");
    }

    #[test]
    fn buffering_a_polygon_grows_it_rather_than_replacing_it_with_a_disc() {
        let square = Polygon::new(
            LineString::from(vec![
                (8.50, 47.37),
                (8.51, 47.37),
                (8.51, 47.38),
                (8.50, 47.38),
                (8.50, 47.37),
            ]),
            vec![],
        );
        let original = wgs(Geometry::Polygon(square));
        let grown = buffer(&original, 100.0, None).unwrap();

        let a0 = super::super::measure::area(&original);
        let a1 = super::super::measure::area(&grown);
        assert!(a1 > a0, "a positive buffer grows the area: {a1} vs {a0}");
        // A 100 m ring around a ~750x1100 m box roughly doubles it, and certainly
        // does not multiply it by ten (which a centroid disc of radius 100 m
        // would not do either, but it would SHRINK it).
        assert!(a1 < a0 * 4.0, "{a1} vs {a0}");
    }

    #[test]
    fn a_negative_buffer_erodes_a_polygon() {
        let big = Polygon::new(
            LineString::from(vec![
                (8.50, 47.37),
                (8.55, 47.37),
                (8.55, 47.40),
                (8.50, 47.40),
                (8.50, 47.37),
            ]),
            vec![],
        );
        let original = wgs(Geometry::Polygon(big));
        let eroded = buffer(&original, -300.0, None).unwrap();
        assert!(
            super::super::measure::area(&eroded) < super::super::measure::area(&original),
            "a negative distance must shrink the shape"
        );
    }

    #[test]
    fn eroding_a_shape_to_nothing_yields_an_empty_geometry_rather_than_an_error() {
        let tiny = wgs(Geometry::Polygon(Polygon::new(
            LineString::from(vec![
                (8.500, 47.370),
                (8.501, 47.370),
                (8.501, 47.371),
                (8.500, 47.370),
            ]),
            vec![],
        )));
        let gone = buffer(&tiny, -10_000.0, None).unwrap();
        assert!(gone.is_empty(), "{:?}", gone.geometry);
    }

    #[test]
    fn every_geometry_type_can_be_buffered() {
        let cases: Vec<Geometry<f64>> = vec![
            Geometry::Point(Point::new(8.54, 47.37)),
            Geometry::MultiPoint(geo::MultiPoint(vec![
                Point::new(8.54, 47.37),
                Point::new(8.55, 47.38),
            ])),
            Geometry::LineString(LineString::from(vec![(8.54, 47.37), (8.55, 47.38)])),
            Geometry::MultiLineString(MultiLineString(vec![LineString::from(vec![
                (8.54, 47.37),
                (8.55, 47.38),
            ])])),
            Geometry::GeometryCollection(vec![Geometry::Point(Point::new(8.54, 47.37))].into()),
        ];
        for g in cases {
            let label = format!("{g:?}");
            let out = buffer(&wgs(g), 50.0, None)
                .unwrap_or_else(|e| panic!("buffering {label} failed: {e}"));
            assert!(
                out.geometry.unsigned_area() > 0.0,
                "{label} produced nothing"
            );
        }
    }

    #[test]
    fn on_a_projected_crs_the_distance_is_native_units_and_no_reprojection_happens() {
        let utm = Crs::from_srid(32632);
        let g = Geom::new(Geometry::Point(Point::new(500_000.0, 5_000_000.0)), utm);
        let disc = buffer(&g, 100.0, None).unwrap();
        assert_eq!(disc.srid, utm);
        let e = disc.envelope().unwrap();
        assert!(
            ((e.max_x - e.min_x) - 200.0).abs() < 5.0,
            "a 100-unit buffer spans ~200 units: {}",
            e.max_x - e.min_x
        );
    }

    #[test]
    fn a_coarser_segment_count_produces_fewer_vertices() {
        let g = wgs(Geometry::Point(Point::new(8.54, 47.37)));
        let count = |segments| match buffer(&g, 1000.0, Some(segments)).unwrap().geometry {
            Geometry::Polygon(p) => p.exterior().0.len(),
            other => panic!("{other:?}"),
        };
        assert!(count(2) < count(16), "{} vs {}", count(2), count(16));
    }

    #[test]
    fn a_zero_or_negative_segment_count_is_rejected() {
        let g = wgs(Geometry::Point(Point::new(8.54, 47.37)));
        assert!(buffer(&g, 100.0, Some(0)).is_err());
        assert!(buffer(&g, 100.0, Some(-3)).is_err());
    }

    #[test]
    fn simplify_tolerance_on_lon_lat_is_metres() {
        // Three near-collinear points 100 m apart; a 500 m tolerance must drop the
        // middle one, while a 1 m tolerance must keep it.
        let jagged = LineString::from(vec![
            (8.5000, 47.3700),
            (8.5013, 47.3701),
            (8.5026, 47.3700),
        ]);
        let g = wgs(Geometry::LineString(jagged));

        let coarse = match simplify(&g, 500.0).unwrap().geometry {
            Geometry::LineString(ls) => ls.0.len(),
            other => panic!("{other:?}"),
        };
        let fine = match simplify(&g, 0.01).unwrap().geometry {
            Geometry::LineString(ls) => ls.0.len(),
            other => panic!("{other:?}"),
        };
        assert_eq!(coarse, 2, "500 m must flatten a 10 m deviation");
        assert_eq!(fine, 3, "1 cm must keep it");
    }

    #[test]
    fn simplify_handles_every_type_and_leaves_points_alone() {
        let point = wgs(Geometry::Point(Point::new(8.54, 47.37)));
        assert_eq!(simplify(&point, 100.0).unwrap().geometry, point.geometry);

        let collection = wgs(Geometry::GeometryCollection(
            vec![
                Geometry::Point(Point::new(8.54, 47.37)),
                Geometry::LineString(LineString::from(vec![
                    (8.50, 47.37),
                    (8.5001, 47.3700001),
                    (8.55, 47.37),
                ])),
            ]
            .into(),
        ));
        assert!(simplify(&collection, 100.0).is_ok());
    }

    #[test]
    fn a_negative_or_nonfinite_tolerance_is_rejected() {
        let g = wgs(Geometry::LineString(LineString::from(vec![
            (0.0, 0.0),
            (1.0, 1.0),
        ])));
        assert!(simplify(&g, -1.0).is_err());
        assert!(simplify(&g, f64::NAN).is_err());
        assert!(buffer(&g, f64::INFINITY, None).is_err());
    }

    #[test]
    fn both_operations_preserve_the_vertical_extent() {
        let g = wgs(Geometry::Point(Point::new(8.54, 47.37))).with_z_range(Some((400.0, 420.0)));
        assert_eq!(
            buffer(&g, 50.0, None).unwrap().z_range,
            Some((400.0, 420.0))
        );
        assert_eq!(simplify(&g, 1.0).unwrap().z_range, Some((400.0, 420.0)));
    }
}
