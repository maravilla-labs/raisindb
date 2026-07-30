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

//! Set operations, tested **per type pair** rather than per function.
//!
//! Testing per function is what let the old implementation ship with
//! Polygon+Polygon working and every other combination returning an error: one
//! test per function passed while eight of nine type pairs were broken.

use super::*;
use geo::{Area, Euclidean, Length, LineString, Point, Polygon};

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

fn poly(x0: f64, y0: f64, side: f64) -> Geom {
    wgs(Geometry::Polygon(square(x0, y0, side)))
}

fn line(coords: Vec<(f64, f64)>) -> Geom {
    wgs(Geometry::LineString(LineString::from(coords)))
}

fn point(x: f64, y: f64) -> Geom {
    wgs(Geometry::Point(Point::new(x, y)))
}

fn run(op: SetOp, a: &Geom, b: &Geom) -> Geometry<f64> {
    apply(op, a, b)
        .expect("a set operation must never fail on valid input")
        .geometry
}

/// The brief's named failure: the union of two disjoint polygons is a
/// MultiPolygon, and `ST_AREA` of it must work.
#[test]
fn union_of_disjoint_polygons_is_a_multipolygon_with_the_summed_area() {
    let g = run(SetOp::Union, &poly(0.0, 0.0, 1.0), &poly(5.0, 0.0, 1.0));
    assert!(matches!(g, Geometry::MultiPolygon(_)), "{g:?}");
    assert!((g.unsigned_area() - 2.0).abs() < 1e-9);
}

#[test]
fn union_of_overlapping_polygons_merges_into_one() {
    let g = run(SetOp::Union, &poly(0.0, 0.0, 2.0), &poly(1.0, 1.0, 2.0));
    assert!(matches!(g, Geometry::Polygon(_)), "{g:?}");
    // 4 + 4 - 1 overlap.
    assert!((g.unsigned_area() - 7.0).abs() < 1e-9);
}

#[test]
fn every_type_pair_produces_a_result_for_all_four_operations() {
    let operands = [
        ("point", point(0.5, 0.5)),
        ("line", line(vec![(0.0, 0.5), (3.0, 0.5)])),
        ("polygon", poly(0.0, 0.0, 1.0)),
        (
            "multipolygon",
            wgs(Geometry::MultiPolygon(MultiPolygon(vec![
                square(0.0, 0.0, 1.0),
                square(4.0, 0.0, 1.0),
            ]))),
        ),
        (
            "collection",
            wgs(Geometry::GeometryCollection(
                vec![
                    Geometry::Point(Point::new(9.0, 9.0)),
                    Geometry::Polygon(square(0.5, 0.0, 1.0)),
                ]
                .into(),
            )),
        ),
        (
            "empty",
            wgs(Geometry::GeometryCollection(Default::default())),
        ),
    ];

    for op in [
        SetOp::Union,
        SetOp::Intersection,
        SetOp::Difference,
        SetOp::SymDifference,
    ] {
        for (na, a) in &operands {
            for (nb, b) in &operands {
                apply(op, a, b).unwrap_or_else(|e| {
                    panic!("{} over ({na}, {nb}) must not fail: {e}", op.name())
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Mixed dimension: the cases that used to be "not supported"
// ---------------------------------------------------------------------------

#[test]
fn intersecting_a_line_with_a_polygon_clips_the_line() {
    // The line spans x in 0..3, the polygon x in 0..1.
    let g = run(
        SetOp::Intersection,
        &line(vec![(-1.0, 0.5), (3.0, 0.5)]),
        &poly(0.0, 0.0, 1.0),
    );
    assert!(
        (Euclidean.length(&as_lines(&g)) - 1.0).abs() < 1e-9,
        "expected the 1.0 inside the polygon, got {g:?}"
    );
}

#[test]
fn differencing_a_line_by_a_polygon_keeps_the_outside_parts() {
    let g = run(
        SetOp::Difference,
        &line(vec![(-1.0, 0.5), (3.0, 0.5)]),
        &poly(0.0, 0.0, 1.0),
    );
    // 1.0 to the left plus 2.0 to the right.
    assert!(
        (Euclidean.length(&as_lines(&g)) - 3.0).abs() < 1e-9,
        "{g:?}"
    );
}

#[test]
fn a_line_through_a_polygon_is_absorbed_by_their_union() {
    let g = run(
        SetOp::Union,
        &poly(0.0, 0.0, 1.0),
        &line(vec![(0.2, 0.5), (0.8, 0.5)]),
    );
    assert!(
        matches!(g, Geometry::Polygon(_)),
        "a fully contained line adds nothing: {g:?}"
    );
}

#[test]
fn a_line_crossing_out_of_a_polygon_leaves_the_outside_stub_in_the_union() {
    let g = run(
        SetOp::Union,
        &poly(0.0, 0.0, 1.0),
        &line(vec![(0.5, 0.5), (3.0, 0.5)]),
    );
    match &g {
        Geometry::GeometryCollection(gc) => {
            assert!(matches!(gc.0[0], Geometry::Polygon(_)));
            assert!(
                (Euclidean.length(&as_lines(&g)) - 2.0).abs() < 1e-6,
                "only the part outside survives: {g:?}"
            );
        }
        other => panic!("expected a mixed collection, got {other:?}"),
    }
}

#[test]
fn point_intersection_keeps_only_the_covered_points() {
    let inside = point(0.5, 0.5);
    let outside = point(9.0, 9.0);
    let square = poly(0.0, 0.0, 1.0);

    assert!(matches!(
        run(SetOp::Intersection, &inside, &square),
        Geometry::Point(_)
    ));
    assert!(
        matches!(
            run(SetOp::Intersection, &outside, &square),
            Geometry::GeometryCollection(gc) if gc.0.is_empty()
        ),
        "a point outside intersects in nothing"
    );
}

#[test]
fn a_point_inside_a_polygon_is_absorbed_by_their_union() {
    let g = run(SetOp::Union, &poly(0.0, 0.0, 1.0), &point(0.5, 0.5));
    assert!(matches!(g, Geometry::Polygon(_)), "{g:?}");
}

#[test]
fn a_point_outside_survives_the_union_as_a_collection_member() {
    let g = run(SetOp::Union, &poly(0.0, 0.0, 1.0), &point(9.0, 9.0));
    match g {
        Geometry::GeometryCollection(gc) => {
            assert_eq!(gc.0.len(), 2);
            assert!(matches!(gc.0[1], Geometry::Point(_)));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn differencing_a_point_by_a_polygon_removes_it_only_if_covered() {
    let square = poly(0.0, 0.0, 1.0);
    assert!(matches!(run(SetOp::Difference, &point(0.5, 0.5), &square),
            Geometry::GeometryCollection(gc) if gc.0.is_empty()));
    assert!(matches!(
        run(SetOp::Difference, &point(9.0, 9.0), &square),
        Geometry::Point(_)
    ));
}

// ---------------------------------------------------------------------------
// Line versus line, which `geo` provides no trait for
// ---------------------------------------------------------------------------

#[test]
fn crossing_lines_intersect_in_a_point() {
    let g = run(
        SetOp::Intersection,
        &line(vec![(0.0, 0.0), (2.0, 0.0)]),
        &line(vec![(1.0, -1.0), (1.0, 1.0)]),
    );
    assert!(matches!(g, Geometry::Point(_)), "{g:?}");
}

#[test]
fn overlapping_lines_intersect_in_a_line() {
    let g = run(
        SetOp::Intersection,
        &line(vec![(0.0, 0.0), (4.0, 0.0)]),
        &line(vec![(1.0, 0.0), (3.0, 0.0)]),
    );
    assert!(
        (Euclidean.length(&as_lines(&g)) - 2.0).abs() < 1e-9,
        "{g:?}"
    );
}

// ---------------------------------------------------------------------------
// Identities and degenerate input
// ---------------------------------------------------------------------------

#[test]
fn symmetric_difference_is_the_union_minus_the_intersection() {
    let (a, b) = (poly(0.0, 0.0, 2.0), poly(1.0, 1.0, 2.0));
    let sym = run(SetOp::SymDifference, &a, &b).unsigned_area();
    let uni = run(SetOp::Union, &a, &b).unsigned_area();
    let inter = run(SetOp::Intersection, &a, &b).unsigned_area();
    assert!(
        (sym - (uni - inter)).abs() < 1e-9,
        "{sym} vs {}",
        uni - inter
    );
}

#[test]
fn a_geometry_differenced_by_itself_is_empty() {
    let a = poly(0.0, 0.0, 1.0);
    assert!(run(SetOp::Difference, &a, &a).is_empty());
    assert!(run(SetOp::SymDifference, &a, &a).is_empty());
}

#[test]
fn the_empty_geometry_is_the_identity_for_union_and_the_annihilator_for_intersection() {
    let a = poly(0.0, 0.0, 1.0);
    let e = wgs(Geometry::GeometryCollection(Default::default()));

    assert!((run(SetOp::Union, &a, &e).unsigned_area() - 1.0).abs() < 1e-9);
    assert!(run(SetOp::Intersection, &a, &e).is_empty());
    assert!((run(SetOp::Difference, &a, &e).unsigned_area() - 1.0).abs() < 1e-9);
    assert!(run(SetOp::Difference, &e, &a).is_empty());
}

#[test]
fn a_projected_crs_survives_the_operation() {
    let utm = raisin_geometry::Crs::from_srid(32632);
    let a = Geom::new(Geometry::Polygon(square(0.0, 0.0, 10.0)), utm);
    let b = Geom::new(Geometry::Polygon(square(5.0, 5.0, 10.0)), utm);
    assert_eq!(apply(SetOp::Union, &a, &b).unwrap().srid, utm);
}

#[test]
fn mismatched_srids_are_an_error_rather_than_a_silent_wrong_answer() {
    let a = Geom::new(
        Geometry::Polygon(square(0.0, 0.0, 1.0)),
        raisin_geometry::Crs::WGS84,
    );
    let b = Geom::new(
        Geometry::Polygon(square(0.0, 0.0, 1.0)),
        raisin_geometry::Crs::WEB_MERCATOR,
    );
    let err = apply(SetOp::Union, &a, &b).unwrap_err().to_string();
    assert!(err.contains("SRID mismatch"), "{err}");
}

#[test]
fn union_all_folds_over_many_operands() {
    let squares: Vec<Geom> = (0..4).map(|i| poly(i as f64 * 3.0, 0.0, 1.0)).collect();
    let g = union_all(squares.iter()).unwrap();
    assert!((g.geometry.unsigned_area() - 4.0).abs() < 1e-9);
    assert!(union_all(std::iter::empty()).is_none());
}

/// Collect whatever linear parts a result has, so a length assertion does not
/// have to care whether the result is a `LineString`, a `MultiLineString` or a
/// mixed collection.
fn as_lines(g: &Geometry<f64>) -> MultiLineString<f64> {
    let mut out = MultiLineString(Vec::new());
    super::super::walk::for_each_line_string(g, &mut |ls| out.0.push(ls.clone()));
    out
}
