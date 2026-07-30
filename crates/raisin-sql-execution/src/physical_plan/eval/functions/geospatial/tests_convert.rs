//! Unit tests for the CRS resolution of a binary geometry pair.
//!
//! Lives beside its siblings under `geospatial/` rather than inside `convert.rs`,
//! which the doc comments for the CRS rule had pushed over the file-size limit.

use super::convert::{geom_arg, geom_pair, narrow_multipolygon};
use crate::physical_plan::executor::Row;
use geo::{Geometry, MultiPolygon};
use raisin_geometry::{from_geo, Crs};
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::{json, Value};

fn arg(v: Value) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Geometry(v)), DataType::Geometry)
}

/// The headline property of the conversion layer, asserted at the SQL
/// boundary rather than only inside `raisin-geometry`: the types the three
/// hand-rolled converters could never reach now arrive intact.
#[test]
fn every_geometry_type_reaches_the_functions() {
    // Ordinates are spelled with an explicit fraction because `serde_json`
    // distinguishes an integer literal from a float one, and the round trip
    // emits floats. That is a JSON-representation detail, not a geometry one.
    for value in [
        json!({"type":"Point","coordinates":[1.0,2.0]}),
        json!({"type":"MultiPoint","coordinates":[[1.0,2.0],[3.0,4.0]]}),
        json!({"type":"LineString","coordinates":[[0.0,0.0],[1.0,1.0]]}),
        json!({"type":"MultiLineString","coordinates":[[[0.0,0.0],[1.0,1.0]]]}),
        json!({"type":"Polygon","coordinates":[[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]]]}),
        json!({"type":"MultiPolygon","coordinates":[[[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,0.0]]]]}),
        json!({"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1.0,2.0]}]}),
    ] {
        let args = vec![arg(value.clone())];
        let g = geom_arg("T", &args, 0, &Row::new())
            .unwrap()
            .expect("not null");
        assert_eq!(
            from_geo(&g).unwrap(),
            value,
            "{} must round trip",
            value["type"]
        );
    }
}

/// An unlabelled operand ends up in the labelled operand's CRS — but by being
/// REPROJECTED out of WGS84, not by having its coordinates reinterpreted.
///
/// The output CRS is the same one the old relabelling rule produced, so
/// nothing downstream changes; what changes is that `(8.54, 47.37)` is now the
/// place in Switzerland it obviously is, instead of a point 8 metres east and
/// 47 metres north of the UTM zone origin.
#[test]
fn an_unlabelled_operand_is_wgs84_and_is_reprojected_into_the_labelled_crs() {
    let args = vec![
        // Zurich in UTM 32N.
        arg(json!({"type":"Point","coordinates":[465270.4,5246384.8],"srid":32632})),
        // The same place in WGS84, unlabelled.
        arg(json!({"type":"Point","coordinates":[8.54,47.37]})),
    ];
    let (a, b) = geom_pair("T", &args, &Row::new()).unwrap().unwrap();
    assert_eq!(a.srid, Crs::from_srid(32632));
    assert_eq!(b.srid, Crs::from_srid(32632), "both operands in one CRS");

    let (geo::Geometry::Point(pa), geo::Geometry::Point(pb)) = (&a.geometry, &b.geometry) else {
        panic!("both are points");
    };
    let metres = ((pa.x() - pb.x()).powi(2) + (pa.y() - pb.y()).powi(2)).sqrt();
    assert!(
        metres < 5.0,
        "the unlabelled WGS84 point must land on the same place, got {metres} m apart"
    );
}

/// The row-level path must agree with the spatial index, which normalises
/// every geometry to WGS84 before deriving its cells.
///
/// This is the regression: `ST_DWITHIN(<EPSG:3857 column>, ST_POINT(8.54,
/// 47.37), 5000)` was `false` on a scan and `true` through the index. It was
/// invisible until a query fell back to a scan mid-rebuild.
#[test]
fn a_wgs84_literal_and_a_web_mercator_geometry_are_the_same_place() {
    let args = vec![
        arg(json!({"type":"Point","coordinates":[950668.45,6002678.0],"srid":3857})),
        arg(json!({"type":"Point","coordinates":[8.54,47.37]})),
    ];
    let (a, b) = geom_pair("ST_DWITHIN", &args, &Row::new())
        .unwrap()
        .unwrap();
    let metres = super::measure::distance(&a, &b).unwrap();
    assert!(
        metres < 5.0,
        "an unlabelled WGS84 centre must be metres from the same place stored \
         in EPSG:3857, got {metres} m"
    );
}

#[test]
fn two_different_explicit_srids_are_an_error_not_an_implicit_transform() {
    let args = vec![
        arg(json!({"type":"Point","coordinates":[1.0,2.0],"srid":4326})),
        arg(json!({"type":"Point","coordinates":[3.0,4.0],"srid":3857})),
    ];
    let err = geom_pair("ST_T", &args, &Row::new())
        .unwrap_err()
        .to_string();
    assert!(err.contains("SRID mismatch"), "{err}");
    assert!(
        err.contains("ST_TRANSFORM"),
        "must say how to fix it: {err}"
    );
}

#[test]
fn null_propagates_from_either_side() {
    let null = TypedExpr::new(Expr::Literal(Literal::Null), DataType::Unknown);
    let point = arg(json!({"type":"Point","coordinates":[1.0,2.0]}));
    assert!(geom_pair("T", &[null.clone(), point.clone()], &Row::new())
        .unwrap()
        .is_none());
    assert!(geom_pair("T", &[point, null], &Row::new())
        .unwrap()
        .is_none());
}

#[test]
fn narrowing_picks_the_minimal_type() {
    use geo::{LineString, Polygon};
    let ring = || {
        Polygon::new(
            LineString::from(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]),
            vec![],
        )
    };
    assert!(matches!(
        narrow_multipolygon(MultiPolygon(vec![])),
        Geometry::GeometryCollection(_)
    ));
    assert!(matches!(
        narrow_multipolygon(MultiPolygon(vec![ring()])),
        Geometry::Polygon(_)
    ));
    assert!(matches!(
        narrow_multipolygon(MultiPolygon(vec![ring(), ring()])),
        Geometry::MultiPolygon(_)
    ));
}
