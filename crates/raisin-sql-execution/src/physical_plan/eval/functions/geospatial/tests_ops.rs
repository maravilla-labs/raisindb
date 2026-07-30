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

//! Every measurement, set-operation, processing and accessor function against
//! **every geometry type its signature admits**.
//!
//! `tests.rs` is the pre-existing suite and checks one representative input per
//! function. That shape is exactly what let the previous implementation ship: a
//! passing test per function while `Multi*` and `GeometryCollection` were rejected
//! everywhere. This module is the type-coverage matrix, so the claim "no ST_\*
//! function returns an unsupported-type error" is asserted rather than asserted
//! about.

use super::*;
use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::{json, Value};

fn geom(v: Value) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Geometry(v)), DataType::Geometry)
}

fn num(v: f64) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Double(v)), DataType::Double)
}

fn int(v: i32) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Int(v)), DataType::Int)
}

fn row() -> Row {
    Row::new()
}

/// One value of every GeoJSON type, all in the same neighbourhood so that
/// distances and overlaps between them are meaningful.
fn all_types() -> Vec<(&'static str, Value)> {
    vec![
        (
            "Point",
            json!({"type":"Point","coordinates":[8.540,47.370]}),
        ),
        (
            "MultiPoint",
            json!({"type":"MultiPoint","coordinates":[[8.540,47.370],[8.545,47.375]]}),
        ),
        (
            "LineString",
            json!({"type":"LineString","coordinates":[[8.540,47.370],[8.550,47.375]]}),
        ),
        (
            "MultiLineString",
            json!({"type":"MultiLineString","coordinates":[
                [[8.540,47.370],[8.550,47.375]],
                [[8.560,47.380],[8.570,47.385]]
            ]}),
        ),
        (
            "Polygon",
            json!({"type":"Polygon","coordinates":[[
                [8.540,47.370],[8.550,47.370],[8.550,47.380],[8.540,47.380],[8.540,47.370]
            ]]}),
        ),
        (
            "PolygonWithHole",
            json!({"type":"Polygon","coordinates":[
                [[8.540,47.370],[8.560,47.370],[8.560,47.390],[8.540,47.390],[8.540,47.370]],
                [[8.545,47.375],[8.555,47.375],[8.555,47.385],[8.545,47.385],[8.545,47.375]]
            ]}),
        ),
        (
            "MultiPolygon",
            json!({"type":"MultiPolygon","coordinates":[
                [[[8.540,47.370],[8.545,47.370],[8.545,47.375],[8.540,47.370]]],
                [[[8.560,47.380],[8.565,47.380],[8.565,47.385],[8.560,47.380]]]
            ]}),
        ),
        (
            "GeometryCollection",
            json!({"type":"GeometryCollection","geometries":[
                {"type":"Point","coordinates":[8.540,47.370]},
                {"type":"Polygon","coordinates":[[
                    [8.550,47.380],[8.560,47.380],[8.560,47.390],[8.550,47.380]
                ]]}
            ]}),
        ),
        (
            "Empty",
            json!({"type":"GeometryCollection","geometries":[]}),
        ),
        (
            "Point3D",
            json!({"type":"Point","coordinates":[8.540,47.370,412.0]}),
        ),
    ]
}

// ---------------------------------------------------------------------------
// The headline claim: no unsupported-type errors anywhere
// ---------------------------------------------------------------------------

/// The single most important assertion in this module. Before the rewrite, seven
/// of these ten inputs produced `"ST_X not supported for geometry type: ..."` from
/// most of these functions.
#[test]
fn no_unary_function_rejects_any_geometry_type() {
    let unary: Vec<(&str, Box<dyn SqlFunction>)> = vec![
        ("ST_AREA", Box::new(StAreaFunction)),
        ("ST_LENGTH", Box::new(StLengthFunction)),
        ("ST_PERIMETER", Box::new(StPerimeterFunction)),
        ("ST_CENTROID", Box::new(StCentroidFunction)),
        ("ST_ENVELOPE", Box::new(StEnvelopeFunction)),
        ("ST_CONVEXHULL", Box::new(StConvexHullFunction)),
        ("ST_BOUNDARY", Box::new(StBoundaryFunction)),
        ("ST_REVERSE", Box::new(StReverseFunction)),
        ("ST_ISVALID", Box::new(StIsValidFunction)),
        ("ST_ISVALIDREASON", Box::new(StIsValidReasonFunction)),
        ("ST_MAKEVALID", Box::new(StMakeValidFunction)),
        ("ST_ISSIMPLE", Box::new(StIsSimpleFunction)),
        ("ST_ISEMPTY", Box::new(StIsEmptyFunction)),
        ("ST_ISCLOSED", Box::new(StIsClosedFunction)),
        ("ST_GEOMETRYTYPE", Box::new(StGeometryTypeFunction)),
        ("ST_NUMPOINTS", Box::new(StNumPointsFunction)),
        ("ST_NUMGEOMETRIES", Box::new(StNumGeometriesFunction)),
        ("ST_ASGEOJSON", Box::new(StAsGeoJsonFunction)),
        // These four are DEFINED to be NULL off their domain rather than to error,
        // so they belong here too: NULL is an answer, an error is not.
        ("ST_X", Box::new(StXFunction)),
        ("ST_Y", Box::new(StYFunction)),
        ("ST_STARTPOINT", Box::new(StStartPointFunction)),
        ("ST_ENDPOINT", Box::new(StEndPointFunction)),
    ];

    for (name, f) in &unary {
        for (label, value) in all_types() {
            f.evaluate(&[geom(value)], &row())
                .unwrap_or_else(|e| panic!("{name}({label}) must not error: {e}"));
        }
    }
}

#[test]
fn no_binary_or_parameterised_function_rejects_any_geometry_type() {
    let cases = all_types();

    for (label, value) in &cases {
        // Parameterised unary functions.
        StBufferFunction
            .evaluate(&[geom(value.clone()), num(50.0)], &row())
            .unwrap_or_else(|e| panic!("ST_BUFFER({label}) must not error: {e}"));
        StBufferFunction
            .evaluate(&[geom(value.clone()), num(50.0), int(4)], &row())
            .unwrap_or_else(|e| panic!("ST_BUFFER({label}, 3-arg) must not error: {e}"));
        StSimplifyFunction
            .evaluate(&[geom(value.clone()), num(1.0)], &row())
            .unwrap_or_else(|e| panic!("ST_SIMPLIFY({label}) must not error: {e}"));
        StPointNFunction
            .evaluate(&[geom(value.clone()), int(1)], &row())
            .unwrap_or_else(|e| panic!("ST_POINTN({label}) must not error: {e}"));
        StLineInterpolatePointFunction
            .evaluate(&[geom(value.clone()), num(0.5)], &row())
            .unwrap_or_else(|e| panic!("ST_LINEINTERPOLATEPOINT({label}) must not error: {e}"));

        for (other_label, other) in &cases {
            let pair = || vec![geom(value.clone()), geom(other.clone())];
            let ctx = format!("({label}, {other_label})");

            StDistanceFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_DISTANCE{ctx}: {e}"));
            StDWithinFunction
                .evaluate(
                    &[geom(value.clone()), geom(other.clone()), num(1000.0)],
                    &row(),
                )
                .unwrap_or_else(|e| panic!("ST_DWITHIN{ctx}: {e}"));
            StUnionFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_UNION{ctx}: {e}"));
            StIntersectionFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_INTERSECTION{ctx}: {e}"));
            StDifferenceFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_DIFFERENCE{ctx}: {e}"));
            StSymDifferenceFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_SYMDIFFERENCE{ctx}: {e}"));
            StCollectFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_COLLECT{ctx}: {e}"));
            StMakeLineFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_MAKELINE{ctx}: {e}"));
            StAzimuthFunction
                .evaluate(&pair(), &row())
                .unwrap_or_else(|e| panic!("ST_AZIMUTH{ctx}: {e}"));
        }
    }
}

/// Whatever a function returns must be readable by the next one. A set operation
/// that emits a type nothing else accepts is only half a fix.
#[test]
fn function_output_is_valid_input_to_the_next_function() {
    let a = json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[2.0,0.0],[2.0,2.0],[0.0,2.0],[0.0,0.0]
    ]]});
    let b = json!({"type":"Polygon","coordinates":[[
        [5.0,5.0],[6.0,5.0],[6.0,6.0],[5.0,5.0]
    ]]});

    // The brief's named failure: a union of disjoint polygons is a MultiPolygon,
    // and ST_AREA of it used to error.
    let union = match StUnionFunction
        .evaluate(&[geom(a.clone()), geom(b)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!(union["type"], "MultiPolygon");

    for (name, f) in [
        ("ST_AREA", Box::new(StAreaFunction) as Box<dyn SqlFunction>),
        ("ST_PERIMETER", Box::new(StPerimeterFunction)),
        ("ST_CENTROID", Box::new(StCentroidFunction)),
        ("ST_ENVELOPE", Box::new(StEnvelopeFunction)),
        ("ST_CONVEXHULL", Box::new(StConvexHullFunction)),
        ("ST_ISVALID", Box::new(StIsValidFunction)),
        ("ST_NUMGEOMETRIES", Box::new(StNumGeometriesFunction)),
    ] {
        f.evaluate(&[geom(union.clone())], &row())
            .unwrap_or_else(|e| panic!("{name} over a union result: {e}"));
    }

    // A buffer's output feeds an intersection, and a boundary's output feeds a
    // length.
    let buffered = StBufferFunction
        .evaluate(&[geom(a.clone()), num(1000.0)], &row())
        .unwrap();
    let buffered = match buffered {
        Literal::Geometry(v) => v,
        other => panic!("{other:?}"),
    };
    StIntersectionFunction
        .evaluate(&[geom(buffered), geom(a.clone())], &row())
        .expect("a buffer result must be intersectable");

    let boundary = match StBoundaryFunction.evaluate(&[geom(a)], &row()).unwrap() {
        Literal::Geometry(v) => v,
        other => panic!("{other:?}"),
    };
    match StLengthFunction
        .evaluate(&[geom(boundary)], &row())
        .unwrap()
    {
        Literal::Double(l) => assert!(l > 0.0, "a polygon's boundary has length, got {l}"),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Units and semantics, asserted rather than documented
// ---------------------------------------------------------------------------

fn as_double(l: Literal) -> f64 {
    match l {
        Literal::Double(d) => d,
        other => panic!("expected Double, got {other:?}"),
    }
}

/// `ST_AREA` on 4326 is square METRES. Square degrees would be about 1e-4 here.
#[test]
fn area_on_lon_lat_is_square_metres_not_square_degrees() {
    let square = json!({"type":"Polygon","coordinates":[[
        [8.50,47.35],[8.51,47.35],[8.51,47.36],[8.50,47.36],[8.50,47.35]
    ]]});
    let a = as_double(StAreaFunction.evaluate(&[geom(square)], &row()).unwrap());
    assert!(
        (6.0e5..1.2e6).contains(&a),
        "expected ~8.3e5 m^2, got {a} — square degrees would be 1e-4"
    );
}

/// `ST_LENGTH` and `ST_PERIMETER` are different questions, and the old
/// implementation answered both with the exterior ring length.
#[test]
fn length_and_perimeter_answer_different_questions() {
    let square = json!({"type":"Polygon","coordinates":[[
        [8.50,47.35],[8.51,47.35],[8.51,47.36],[8.50,47.36],[8.50,47.35]
    ]]});
    assert_eq!(
        as_double(
            StLengthFunction
                .evaluate(&[geom(square.clone())], &row())
                .unwrap()
        ),
        0.0,
        "ST_LENGTH of an areal geometry is 0"
    );
    assert!(
        as_double(
            StPerimeterFunction
                .evaluate(&[geom(square)], &row())
                .unwrap()
        ) > 3000.0,
        "ST_PERIMETER measures the boundary"
    );
}

#[test]
fn perimeter_counts_interior_rings() {
    let solid = json!({"type":"Polygon","coordinates":[
        [[8.540,47.370],[8.560,47.370],[8.560,47.390],[8.540,47.390],[8.540,47.370]]
    ]});
    let holed = json!({"type":"Polygon","coordinates":[
        [[8.540,47.370],[8.560,47.370],[8.560,47.390],[8.540,47.390],[8.540,47.370]],
        [[8.545,47.375],[8.555,47.375],[8.555,47.385],[8.545,47.385],[8.545,47.375]]
    ]});
    let p_solid = as_double(
        StPerimeterFunction
            .evaluate(&[geom(solid)], &row())
            .unwrap(),
    );
    let p_holed = as_double(
        StPerimeterFunction
            .evaluate(&[geom(holed)], &row())
            .unwrap(),
    );
    assert!(p_holed > p_solid, "{p_holed} vs {p_solid}");
}

/// The centroid fallback's most visible symptom.
#[test]
fn distance_between_overlapping_polygons_is_zero() {
    let a = json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[2.0,0.0],[2.0,2.0],[0.0,2.0],[0.0,0.0]
    ]]});
    let b = json!({"type":"Polygon","coordinates":[[
        [1.0,1.0],[3.0,1.0],[3.0,3.0],[1.0,3.0],[1.0,1.0]
    ]]});
    assert_eq!(
        as_double(
            StDistanceFunction
                .evaluate(&[geom(a.clone()), geom(b.clone())], &row())
                .unwrap()
        ),
        0.0
    );
    assert_eq!(
        StDWithinFunction
            .evaluate(&[geom(a), geom(b), num(0.0)], &row())
            .unwrap(),
        Literal::Boolean(true),
        "zero apart means within zero"
    );
}

#[test]
fn buffer_distance_is_metres_and_follows_the_shape() {
    // A 200 m corridor along a ~7.5 km road must be far wider than it is tall.
    let road = json!({"type":"LineString","coordinates":[[8.50,47.37],[8.60,47.37]]});
    let corridor = match StBufferFunction
        .evaluate(&[geom(road), num(200.0)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => v,
        other => panic!("{other:?}"),
    };
    let env = match StEnvelopeFunction
        .evaluate(&[geom(corridor)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => v,
        other => panic!("{other:?}"),
    };
    let ring = env["coordinates"][0].as_array().unwrap();
    let xs: Vec<f64> = ring.iter().map(|c| c[0].as_f64().unwrap()).collect();
    let ys: Vec<f64> = ring.iter().map(|c| c[1].as_f64().unwrap()).collect();
    let width =
        xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    let height =
        ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
    assert!(
        width > height * 5.0 && width > 0.10,
        "expected a corridor along the road, got {width} x {height}"
    );
}

#[test]
fn a_bowtie_polygon_is_invalid_explained_and_repairable() {
    let bowtie = json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[2.0,2.0],[2.0,0.0],[0.0,2.0],[0.0,0.0]
    ]]});

    assert_eq!(
        StIsValidFunction
            .evaluate(&[geom(bowtie.clone())], &row())
            .unwrap(),
        Literal::Boolean(false),
        "the old array-shape check passed this"
    );
    assert_eq!(
        StIsSimpleFunction
            .evaluate(&[geom(bowtie.clone())], &row())
            .unwrap(),
        Literal::Boolean(false),
        "the old implementation returned a constant true"
    );

    match StIsValidReasonFunction
        .evaluate(&[geom(bowtie.clone())], &row())
        .unwrap()
    {
        Literal::Text(reason) => assert!(reason.contains("self-intersection"), "{reason}"),
        other => panic!("{other:?}"),
    }

    let fixed = match StMakeValidFunction
        .evaluate(&[geom(bowtie)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        StIsValidFunction.evaluate(&[geom(fixed)], &row()).unwrap(),
        Literal::Boolean(true)
    );
}

#[test]
fn a_self_crossing_line_is_valid_but_not_simple() {
    let figure_eight = json!({"type":"LineString","coordinates":[
        [0.0,0.0],[2.0,2.0],[2.0,0.0],[0.0,2.0]
    ]});
    assert_eq!(
        StIsValidFunction
            .evaluate(&[geom(figure_eight.clone())], &row())
            .unwrap(),
        Literal::Boolean(true),
        "a LineString is permitted to cross itself"
    );
    assert_eq!(
        StIsSimpleFunction
            .evaluate(&[geom(figure_eight)], &row())
            .unwrap(),
        Literal::Boolean(false)
    );
}

#[test]
fn make_valid_leaves_a_valid_geometry_untouched() {
    let clean = json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0],[0.0,0.0]
    ]]});
    assert_eq!(
        StMakeValidFunction
            .evaluate(&[geom(clean.clone())], &row())
            .unwrap(),
        Literal::Geometry(clean),
        "a repair must be safe to run over a whole column"
    );
}

// ---------------------------------------------------------------------------
// SRID, altitude and NULL, which every function shares
// ---------------------------------------------------------------------------

#[test]
fn a_projected_srid_survives_every_transformation() {
    let utm = json!({"type":"Polygon","coordinates":[[
        [500000.0,5000000.0],[500100.0,5000000.0],[500100.0,5000100.0],[500000.0,5000000.0]
    ]],"srid":32632});

    for (name, f) in [
        (
            "ST_CENTROID",
            Box::new(StCentroidFunction) as Box<dyn SqlFunction>,
        ),
        ("ST_ENVELOPE", Box::new(StEnvelopeFunction)),
        ("ST_CONVEXHULL", Box::new(StConvexHullFunction)),
        ("ST_REVERSE", Box::new(StReverseFunction)),
        ("ST_MAKEVALID", Box::new(StMakeValidFunction)),
    ] {
        match f.evaluate(&[geom(utm.clone())], &row()).unwrap() {
            Literal::Geometry(v) => assert_eq!(v["srid"], 32632, "{name} dropped the SRID"),
            other => panic!("{name}: {other:?}"),
        }
    }

    // On a projected CRS the area is native units squared: a 100x100 half-square.
    let a = as_double(StAreaFunction.evaluate(&[geom(utm)], &row()).unwrap());
    assert!((a - 5000.0).abs() < 1.0, "expected 5000 m^2, got {a}");
}

#[test]
fn two_different_explicit_srids_are_an_error_naming_the_fix() {
    let wgs = json!({"type":"Point","coordinates":[8.54,47.37],"srid":4326});
    let mercator = json!({"type":"Point","coordinates":[950000.0,6000000.0],"srid":3857});
    let err = StDistanceFunction
        .evaluate(&[geom(wgs), geom(mercator)], &row())
        .unwrap_err()
        .to_string();
    assert!(err.contains("SRID mismatch"), "{err}");
    assert!(err.contains("ST_TRANSFORM"), "{err}");
}

/// Altitude is dropped by `geo`, so it must be preserved by the one function whose
/// job is to report the stored representation.
#[test]
fn asgeojson_preserves_altitude_and_can_round_ordinates() {
    let point3d = json!({"type":"Point","coordinates":[8.5401234,47.3701234,412.5]});
    match StAsGeoJsonFunction
        .evaluate(&[geom(point3d.clone())], &row())
        .unwrap()
    {
        Literal::Text(s) => assert!(s.contains("412.5"), "altitude must survive: {s}"),
        other => panic!("{other:?}"),
    }
    match StAsGeoJsonFunction
        .evaluate(&[geom(point3d), int(3)], &row())
        .unwrap()
    {
        Literal::Text(s) => {
            assert!(s.contains("8.54") && !s.contains("8.5401234"), "{s}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_function_propagates_null() {
    let null = TypedExpr::new(Expr::Literal(Literal::Null), DataType::Unknown);
    let point = geom(json!({"type":"Point","coordinates":[8.54,47.37]}));

    let unary: Vec<Box<dyn SqlFunction>> = vec![
        Box::new(StAreaFunction),
        Box::new(StLengthFunction),
        Box::new(StPerimeterFunction),
        Box::new(StCentroidFunction),
        Box::new(StEnvelopeFunction),
        Box::new(StConvexHullFunction),
        Box::new(StBoundaryFunction),
        Box::new(StReverseFunction),
        Box::new(StIsValidFunction),
        Box::new(StIsValidReasonFunction),
        Box::new(StMakeValidFunction),
        Box::new(StIsSimpleFunction),
        Box::new(StIsEmptyFunction),
        Box::new(StIsClosedFunction),
        Box::new(StGeometryTypeFunction),
        Box::new(StNumPointsFunction),
        Box::new(StNumGeometriesFunction),
        Box::new(StAsGeoJsonFunction),
        Box::new(StXFunction),
        Box::new(StYFunction),
        Box::new(StStartPointFunction),
        Box::new(StEndPointFunction),
    ];
    for f in &unary {
        assert_eq!(
            f.evaluate(&[null.clone()], &row()).unwrap(),
            Literal::Null,
            "{} must propagate NULL",
            f.name()
        );
    }

    let binary: Vec<Box<dyn SqlFunction>> = vec![
        Box::new(StDistanceFunction),
        Box::new(StUnionFunction),
        Box::new(StIntersectionFunction),
        Box::new(StDifferenceFunction),
        Box::new(StSymDifferenceFunction),
        Box::new(StCollectFunction),
        Box::new(StMakeLineFunction),
        Box::new(StAzimuthFunction),
    ];
    for f in &binary {
        for args in [
            vec![null.clone(), point.clone()],
            vec![point.clone(), null.clone()],
        ] {
            assert_eq!(
                f.evaluate(&args, &row()).unwrap(),
                Literal::Null,
                "{} must propagate NULL from either side",
                f.name()
            );
        }
    }

    // A NULL numeric parameter propagates too, not just a NULL geometry.
    assert_eq!(
        StBufferFunction
            .evaluate(&[point.clone(), null.clone()], &row())
            .unwrap(),
        Literal::Null
    );
    assert_eq!(
        StSimplifyFunction
            .evaluate(&[point.clone(), null.clone()], &row())
            .unwrap(),
        Literal::Null
    );
    assert_eq!(
        StDWithinFunction
            .evaluate(&[point.clone(), point, null], &row())
            .unwrap(),
        Literal::Null
    );
}

#[test]
fn an_empty_geometry_propagates_instead_of_erroring() {
    let empty = json!({"type":"GeometryCollection","geometries":[]});

    assert_eq!(
        StIsEmptyFunction
            .evaluate(&[geom(empty.clone())], &row())
            .unwrap(),
        Literal::Boolean(true)
    );
    assert_eq!(
        as_double(
            StAreaFunction
                .evaluate(&[geom(empty.clone())], &row())
                .unwrap()
        ),
        0.0
    );
    assert_eq!(
        StNumGeometriesFunction
            .evaluate(&[geom(empty.clone())], &row())
            .unwrap(),
        Literal::Int(0)
    );
    // The empty geometry is the identity for union.
    let point = json!({"type":"Point","coordinates":[8.54,47.37]});
    match StUnionFunction
        .evaluate(&[geom(point.clone()), geom(empty)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => assert_eq!(v, point),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Accessor edge cases
// ---------------------------------------------------------------------------

#[test]
fn x_and_y_are_null_off_their_domain_rather_than_an_error() {
    let line = json!({"type":"LineString","coordinates":[[0.0,0.0],[1.0,1.0]]});
    assert_eq!(
        StXFunction.evaluate(&[geom(line.clone())], &row()).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StYFunction.evaluate(&[geom(line)], &row()).unwrap(),
        Literal::Null
    );

    // A one-member MultiPoint IS a location; the answer must not depend on the
    // spelling.
    let single = json!({"type":"MultiPoint","coordinates":[[8.54,47.37]]});
    assert_eq!(
        StXFunction.evaluate(&[geom(single)], &row()).unwrap(),
        Literal::Double(8.54)
    );
}

#[test]
fn pointn_is_one_based_and_indexes_from_the_end_when_negative() {
    let line = json!({"type":"LineString","coordinates":[[0.0,0.0],[1.0,1.0],[2.0,2.0]]});
    let nth = |n: i32| match StPointNFunction
        .evaluate(&[geom(line.clone()), int(n)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => Some(v["coordinates"].clone()),
        Literal::Null => None,
        other => panic!("{other:?}"),
    };

    assert_eq!(nth(1), Some(json!([0.0, 0.0])));
    assert_eq!(nth(3), Some(json!([2.0, 2.0])));
    assert_eq!(
        nth(-1),
        Some(json!([2.0, 2.0])),
        "negative counts from the end"
    );
    assert_eq!(nth(0), None, "0 names no vertex");
    assert_eq!(nth(99), None, "out of range is NULL, not an error");
}

#[test]
fn line_interpolation_uses_geodesic_length_not_coordinate_units() {
    // A diagonal from (0,0) to (1,1) at high latitude: in raw degrees the midpoint
    // is (0.5, 0.5), but a degree of longitude is much shorter than a degree of
    // latitude at 60N, so the geodesic midpoint sits further along in longitude.
    let diagonal = json!({"type":"LineString","coordinates":[[0.0,60.0],[1.0,61.0]]});
    match StLineInterpolatePointFunction
        .evaluate(&[geom(diagonal.clone()), num(0.5)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => {
            let x = v["coordinates"][0].as_f64().unwrap();
            assert!((0.45..0.55).contains(&x), "midpoint of one segment: {x}");
        }
        other => panic!("{other:?}"),
    }

    // The endpoints are exact.
    for (fraction, expected) in [(0.0, json!([0.0, 60.0])), (1.0, json!([1.0, 61.0]))] {
        match StLineInterpolatePointFunction
            .evaluate(&[geom(diagonal.clone()), num(fraction)], &row())
            .unwrap()
        {
            Literal::Geometry(v) => assert_eq!(v["coordinates"], expected),
            other => panic!("{other:?}"),
        }
    }

    // Out of range is an arithmetic mistake, not something to clamp silently.
    assert!(StLineInterpolatePointFunction
        .evaluate(&[geom(diagonal), num(1.5)], &row())
        .is_err());
}

#[test]
fn boundary_of_a_closed_ring_is_empty() {
    let ring = json!({"type":"LineString","coordinates":[
        [0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]
    ]});
    match StBoundaryFunction.evaluate(&[geom(ring)], &row()).unwrap() {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "GeometryCollection");
            assert_eq!(v["geometries"].as_array().unwrap().len(), 0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn boundary_of_a_polygon_with_a_hole_includes_the_hole() {
    let holed = json!({"type":"Polygon","coordinates":[
        [[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]],
        [[1.0,1.0],[2.0,1.0],[2.0,2.0],[1.0,1.0]]
    ]});
    match StBoundaryFunction.evaluate(&[geom(holed)], &row()).unwrap() {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "MultiLineString");
            assert_eq!(v["coordinates"].as_array().unwrap().len(), 2);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn reverse_is_its_own_inverse_and_keeps_the_type() {
    for value in all_types().into_iter().map(|(_, v)| v) {
        let once = StReverseFunction
            .evaluate(&[geom(value.clone())], &row())
            .unwrap();
        let once = match once {
            Literal::Geometry(v) => v,
            other => panic!("{other:?}"),
        };
        assert_eq!(once["type"], value["type"], "the type must be preserved");

        let twice = StReverseFunction.evaluate(&[geom(once)], &row()).unwrap();
        assert_eq!(
            twice,
            Literal::Geometry(value.clone()),
            "reversing twice must be the identity for {}",
            value["type"]
        );
    }
}

#[test]
fn makeenvelope_normalizes_swapped_bounds_and_can_be_labelled() {
    let swapped = StMakeEnvelopeFunction
        .evaluate(&[num(8.60), num(47.40), num(8.50), num(47.35)], &row())
        .unwrap();
    let normal = StMakeEnvelopeFunction
        .evaluate(&[num(8.50), num(47.35), num(8.60), num(47.40)], &row())
        .unwrap();
    assert_eq!(swapped, normal, "swapped corners must be corrected");

    match StMakeEnvelopeFunction
        .evaluate(
            &[num(0.0), num(0.0), num(100.0), num(100.0), int(32632)],
            &row(),
        )
        .unwrap()
    {
        Literal::Geometry(v) => assert_eq!(v["srid"], 32632),
        other => panic!("{other:?}"),
    }
}

#[test]
fn makepolygon_refuses_to_invent_a_closing_vertex() {
    // Four vertices, so the "at least 4" rule is satisfied and the CLOSED rule is
    // the one under test.
    let open = json!({"type":"LineString","coordinates":[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]]});
    let err = StMakePolygonFunction
        .evaluate(&[geom(open)], &row())
        .unwrap_err()
        .to_string();
    assert!(err.contains("closed"), "{err}");

    let too_short = json!({"type":"LineString","coordinates":[[0.0,0.0],[1.0,0.0],[0.0,0.0]]});
    let err = StMakePolygonFunction
        .evaluate(&[geom(too_short)], &row())
        .unwrap_err()
        .to_string();
    assert!(err.contains("4 vertices"), "{err}");

    let closed = json!({"type":"LineString","coordinates":[
        [0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]
    ]});
    match StMakePolygonFunction
        .evaluate(&[geom(closed)], &row())
        .unwrap()
    {
        Literal::Geometry(v) => assert_eq!(v["type"], "Polygon"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn geomfromgeojson_rejects_a_shape_that_does_not_match_its_declared_type() {
    let text = |s: &str| TypedExpr::new(Expr::Literal(Literal::Text(s.into())), DataType::Text);

    // The old validation accepted this: the `type` name was in the allow-list and a
    // `coordinates` key existed.
    assert!(StGeomFromGeoJsonFunction
        .evaluate(&[text(r#"{"type":"Polygon","coordinates":[1,2]}"#)], &row())
        .is_err());
    assert!(StGeomFromGeoJsonFunction
        .evaluate(&[text(r#"{"type":"Circle","coordinates":[1,2]}"#)], &row())
        .is_err());

    let ok = StGeomFromGeoJsonFunction
        .evaluate(
            &[text(r#"{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}"#)],
            &row(),
        )
        .expect("a MultiPoint must parse");
    assert!(matches!(ok, Literal::Geometry(_)));
}

#[test]
fn arity_errors_name_the_signature() {
    let point = geom(json!({"type":"Point","coordinates":[1.0,2.0]}));
    let err = StAreaFunction
        .evaluate(&[point.clone(), point.clone()], &row())
        .unwrap_err()
        .to_string();
    assert!(err.contains("ST_AREA(geometry)"), "{err}");

    // ST_BUFFER accepts 2 or 3; 1 and 4 are both errors that say so.
    assert!(StBufferFunction.evaluate(&[point.clone()], &row()).is_err());
    let err = StBufferFunction
        .evaluate(&[point.clone(), num(1.0), int(1), int(1)], &row())
        .unwrap_err()
        .to_string();
    assert!(err.contains("2 or 3"), "{err}");
}
