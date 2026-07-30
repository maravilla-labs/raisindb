//! Unit tests for the three-dimensional ST_* functions and the axis-order guard.
//!
//! These are unit tests, which are explicitly **not** accepted as proof that the
//! feature works — that is `spatial_measures_test.rs`'s job, against a real
//! server. What they do pin down is the semantics each function promises, in
//! particular the NULL-vs-error decisions, which are easy to get subtly wrong and
//! very hard to notice from an end-to-end test that only uses 3-D data.

use super::*;
use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::json;

fn geom_arg(geojson: serde_json::Value) -> TypedExpr {
    TypedExpr::new(
        Expr::Literal(Literal::Geometry(geojson)),
        DataType::Geometry,
    )
}

fn double_arg(v: f64) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Double(v)), DataType::Double)
}

fn null_arg() -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Null), DataType::Unknown)
}

fn row() -> Row {
    Row::new()
}

fn point_2d() -> serde_json::Value {
    json!({"type": "Point", "coordinates": [8.54, 47.37]})
}

fn point_3d(z: f64) -> serde_json::Value {
    json!({"type": "Point", "coordinates": [8.54, 47.37, z]})
}

// --- ST_Z / ST_ZMIN / ST_ZMAX / ST_NDIMS -------------------------------------

#[test]
fn st_z_returns_the_altitude_of_a_3d_point() {
    let out = StZFunction
        .evaluate(&[geom_arg(point_3d(412.5))], &row())
        .unwrap();
    assert_eq!(out, Literal::Double(412.5));
}

/// The decision that matters: NULL, not an error. A query over a column with a
/// mix of 2-D and 3-D rows must not blow up on the first flat row, which is why
/// PostGIS returns NULL here too.
#[test]
fn st_z_is_null_for_a_2d_point_and_for_any_non_point() {
    assert_eq!(
        StZFunction
            .evaluate(&[geom_arg(point_2d())], &row())
            .unwrap(),
        Literal::Null
    );
    let line = json!({"type":"LineString","coordinates":[[0,0,5.0],[1,1,6.0]]});
    assert_eq!(
        StZFunction.evaluate(&[geom_arg(line)], &row()).unwrap(),
        Literal::Null,
        "ST_Z of a non-Point is NULL even when it has altitude"
    );
}

#[test]
fn zmin_and_zmax_span_a_whole_geometry_unlike_st_z() {
    let poly = json!({"type":"Polygon","coordinates":[[
        [0,0,10.0],[1,0,-2.5],[1,1,30.0],[0,0,10.0]
    ]]});
    assert_eq!(
        StZMinFunction
            .evaluate(&[geom_arg(poly.clone())], &row())
            .unwrap(),
        Literal::Double(-2.5)
    );
    assert_eq!(
        StZMaxFunction.evaluate(&[geom_arg(poly)], &row()).unwrap(),
        Literal::Double(30.0)
    );
}

#[test]
fn zmin_and_zmax_are_null_for_a_flat_geometry() {
    let flat = json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]});
    assert_eq!(
        StZMinFunction
            .evaluate(&[geom_arg(flat.clone())], &row())
            .unwrap(),
        Literal::Null
    );
    assert_eq!(
        StZMaxFunction.evaluate(&[geom_arg(flat)], &row()).unwrap(),
        Literal::Null
    );
}

/// A mixed geometry reports 3, so `ST_NDIMS(g) = 3` is a reliable "this row has
/// altitude data" predicate.
#[test]
fn ndims_reports_three_if_any_vertex_has_altitude() {
    let mixed = json!({"type":"LineString","coordinates":[[0,0],[1,1,9.0]]});
    assert_eq!(
        StNDimsFunction
            .evaluate(&[geom_arg(mixed)], &row())
            .unwrap(),
        Literal::Int(3)
    );
    assert_eq!(
        StNDimsFunction
            .evaluate(&[geom_arg(point_2d())], &row())
            .unwrap(),
        Literal::Int(2)
    );
}

#[test]
fn ndims_walks_into_nested_collections() {
    let coll = json!({"type":"GeometryCollection","geometries":[
        {"type":"Point","coordinates":[0,0]},
        {"type":"GeometryCollection","geometries":[
            {"type":"Point","coordinates":[1,1,3.0]}
        ]}
    ]});
    assert_eq!(
        StNDimsFunction.evaluate(&[geom_arg(coll)], &row()).unwrap(),
        Literal::Int(3)
    );
}

// --- ST_FORCE2D / ST_FORCE3D --------------------------------------------------

#[test]
fn force2d_strips_altitude_and_keeps_everything_else() {
    let v = json!({"type":"Point","coordinates":[8.54,47.37,412.0],"srid":2056});
    let out = StForce2DFunction.evaluate(&[geom_arg(v)], &row()).unwrap();
    match out {
        Literal::Geometry(g) => {
            assert_eq!(g["coordinates"], json!([8.54, 47.37]));
            assert_eq!(g["srid"], 2056, "srid must survive");
        }
        other => panic!("expected Geometry, got {other:?}"),
    }
}

#[test]
fn force3d_fills_only_the_missing_ordinates() {
    let v = json!({"type":"LineString","coordinates":[[0,0],[1,1,9.0]]});
    let out = StForce3DFunction
        .evaluate(&[geom_arg(v), double_arg(5.0)], &row())
        .unwrap();
    match out {
        Literal::Geometry(g) => assert_eq!(
            g["coordinates"],
            json!([[0, 0, 5.0], [1, 1, 9.0]]),
            "an existing Z must not be overwritten"
        ),
        other => panic!("expected Geometry, got {other:?}"),
    }
}

#[test]
fn force3d_rejects_a_non_finite_z() {
    let err = StForce3DFunction
        .evaluate(&[geom_arg(point_2d()), double_arg(f64::NAN)], &row())
        .unwrap_err();
    assert!(err.to_string().contains("finite"), "{err}");
}

// --- ST_3DDISTANCE / ST_3DDWITHIN --------------------------------------------

/// Two points at the same lon/lat 100 m apart vertically: the horizontal leg is
/// zero, so the answer is exactly the vertical gap.
#[test]
fn three_d_distance_of_a_purely_vertical_pair_is_the_height_difference() {
    let out = St3DDistanceFunction
        .evaluate(
            &[geom_arg(point_3d(0.0)), geom_arg(point_3d(100.0))],
            &row(),
        )
        .unwrap();
    match out {
        Literal::Double(d) => assert!((d - 100.0).abs() < 1e-6, "{d}"),
        other => panic!("expected Double, got {other:?}"),
    }
}

/// The Pythagorean composition, on a pair whose horizontal separation is known.
#[test]
fn three_d_distance_is_hypot_of_horizontal_and_vertical() {
    let a = json!({"type":"Point","coordinates":[0.0, 0.0, 0.0]});
    let b = json!({"type":"Point","coordinates":[0.0, 0.001, 0.0]});
    let flat = match St3DDistanceFunction
        .evaluate(&[geom_arg(a.clone()), geom_arg(b.clone())], &row())
        .unwrap()
    {
        Literal::Double(d) => d,
        other => panic!("{other:?}"),
    };
    // ~111 m for a thousandth of a degree of latitude.
    assert!((100.0..125.0).contains(&flat), "horizontal leg was {flat}");

    let b_high = json!({"type":"Point","coordinates":[0.0, 0.001, 200.0]});
    let raised = match St3DDistanceFunction
        .evaluate(&[geom_arg(a), geom_arg(b_high)], &row())
        .unwrap()
    {
        Literal::Double(d) => d,
        other => panic!("{other:?}"),
    };
    assert!(
        (raised - flat.hypot(200.0)).abs() < 1e-6,
        "{raised} != hypot({flat}, 200)"
    );
}

/// Overlapping altitude intervals mean zero vertical separation, so a point
/// inside a tall building's band is vertically coincident with it.
#[test]
fn overlapping_altitude_bands_contribute_no_vertical_distance() {
    let tower = json!({"type":"LineString","coordinates":[[0.0,0.0,0.0],[0.0,0.0,120.0]]});
    let inside = json!({"type":"Point","coordinates":[0.0,0.0,50.0]});
    let out = St3DDistanceFunction
        .evaluate(&[geom_arg(tower), geom_arg(inside)], &row())
        .unwrap();
    match out {
        Literal::Double(d) => assert!(d.abs() < 1e-6, "{d}"),
        other => panic!("expected Double, got {other:?}"),
    }
}

/// NULL rather than pretending the 2-D operand sits at altitude zero — that
/// would silently answer a different question.
#[test]
fn three_d_distance_is_null_when_either_operand_is_flat() {
    for (a, b) in [
        (point_2d(), point_3d(10.0)),
        (point_3d(10.0), point_2d()),
        (point_2d(), point_2d()),
    ] {
        assert_eq!(
            St3DDistanceFunction
                .evaluate(&[geom_arg(a), geom_arg(b)], &row())
                .unwrap(),
            Literal::Null
        );
    }
}

#[test]
fn three_d_dwithin_agrees_with_three_d_distance() {
    let args = |d: f64| {
        vec![
            geom_arg(point_3d(0.0)),
            geom_arg(point_3d(100.0)),
            double_arg(d),
        ]
    };
    assert_eq!(
        St3DDWithinFunction.evaluate(&args(100.0), &row()).unwrap(),
        Literal::Boolean(true),
        "the boundary is inclusive, matching ST_DWITHIN"
    );
    assert_eq!(
        St3DDWithinFunction.evaluate(&args(99.9), &row()).unwrap(),
        Literal::Boolean(false)
    );
}

#[test]
fn three_d_dwithin_rejects_a_negative_radius() {
    let err = St3DDWithinFunction
        .evaluate(
            &[
                geom_arg(point_3d(0.0)),
                geom_arg(point_3d(1.0)),
                double_arg(-1.0),
            ],
            &row(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("non-negative"), "{err}");
}

// --- NULL and arity behaviour, shared by all of them -------------------------

#[test]
fn null_input_propagates_as_null_everywhere() {
    assert_eq!(
        StZFunction.evaluate(&[null_arg()], &row()).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StNDimsFunction.evaluate(&[null_arg()], &row()).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StForce2DFunction.evaluate(&[null_arg()], &row()).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StForce3DFunction
            .evaluate(&[geom_arg(point_2d()), null_arg()], &row())
            .unwrap(),
        Literal::Null
    );
    assert_eq!(
        St3DDistanceFunction
            .evaluate(&[null_arg(), geom_arg(point_3d(1.0))], &row())
            .unwrap(),
        Literal::Null
    );
    assert_eq!(
        St3DDWithinFunction
            .evaluate(
                &[geom_arg(point_3d(0.0)), geom_arg(point_3d(1.0)), null_arg()],
                &row()
            )
            .unwrap(),
        Literal::Null
    );
}

#[test]
fn wrong_arity_names_the_signature() {
    let err = StZFunction.evaluate(&[], &row()).unwrap_err().to_string();
    assert!(err.contains("ST_Z(point)"), "{err}");
}

// --- the axis-order guard, through ST_POINT ----------------------------------

#[test]
fn st_point_rejects_an_unambiguously_reversed_pair() {
    let err = StPointFunction
        .evaluate(&[double_arg(47.37), double_arg(185.4)], &row())
        .unwrap_err()
        .to_string();
    assert!(err.contains("looks reversed"), "{err}");
    assert!(
        err.contains("ST_POINT(185.4, 47.37)"),
        "the message must show the fix: {err}"
    );
}

#[test]
fn st_point_and_st_makepoint_accept_the_same_pairs() {
    for (lon, lat) in [(8.54, 47.37), (-180.0, -90.0), (180.0, 90.0)] {
        assert!(
            StPointFunction
                .evaluate(&[double_arg(lon), double_arg(lat)], &row())
                .is_ok(),
            "ST_POINT({lon}, {lat})"
        );
        assert!(
            StMakePointFunction
                .evaluate(&[double_arg(lon), double_arg(lat)], &row())
                .is_ok(),
            "ST_MAKEPOINT({lon}, {lat})"
        );
    }
    // ...and reject the same ones. The alias must not drift from the original.
    assert!(StPointFunction
        .evaluate(&[double_arg(47.37), double_arg(185.4)], &row())
        .is_err());
    assert!(StMakePointFunction
        .evaluate(&[double_arg(47.37), double_arg(185.4)], &row())
        .is_err());
}
