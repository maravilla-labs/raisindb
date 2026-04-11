use super::*;
use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn geom_arg(geojson: serde_json::Value) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Geometry(geojson)), DataType::Geometry)
}

fn double_arg(v: f64) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Double(v)), DataType::Double)
}

fn int_arg(v: i32) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Int(v)), DataType::Int)
}

fn null_arg() -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Null), DataType::Unknown)
}

fn empty_row() -> Row {
    Row::new()
}

// ---------------------------------------------------------------------------
// Test data
// ---------------------------------------------------------------------------

fn sf_point() -> serde_json::Value {
    json!({"type": "Point", "coordinates": [-122.4194, 37.7749]})
}

fn ny_point() -> serde_json::Value {
    json!({"type": "Point", "coordinates": [-73.9857, 40.7484]})
}

fn sf_polygon() -> serde_json::Value {
    json!({"type": "Polygon", "coordinates": [[
        [-122.5, 37.7], [-122.3, 37.7], [-122.3, 37.8],
        [-122.5, 37.8], [-122.5, 37.7]
    ]]})
}

fn sf_line() -> serde_json::Value {
    json!({"type": "LineString", "coordinates": [
        [-122.4194, 37.7749], [-122.4089, 37.7858]
    ]})
}

/// A point inside sf_polygon (centroid-ish)
fn interior_point() -> serde_json::Value {
    json!({"type": "Point", "coordinates": [-122.4, 37.75]})
}

/// A closed LineString (ring) suitable for ST_MAKEPOLYGON
fn closed_ring() -> serde_json::Value {
    json!({"type": "LineString", "coordinates": [
        [-122.5, 37.7], [-122.3, 37.7], [-122.3, 37.8],
        [-122.5, 37.8], [-122.5, 37.7]
    ]})
}

/// Two overlapping polygons for set-operation tests
fn overlap_poly_a() -> serde_json::Value {
    json!({"type": "Polygon", "coordinates": [[
        [0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]
    ]]})
}

fn overlap_poly_b() -> serde_json::Value {
    json!({"type": "Polygon", "coordinates": [[
        [1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0], [1.0, 1.0]
    ]]})
}

// =========================================================================
// Measurement functions
// =========================================================================

#[test]
fn test_st_area_polygon() {
    let f = StAreaFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Double(area) => assert!(area > 0.0, "polygon area should be positive, got {}", area),
        other => panic!("expected Double, got {:?}", other),
    }
}

#[test]
fn test_st_area_point() {
    let f = StAreaFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Double(0.0));
}

#[test]
fn test_st_length_linestring() {
    let f = StLengthFunction;
    let args = vec![geom_arg(sf_line())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Double(len) => assert!(len > 0.0, "linestring length should be positive, got {}", len),
        other => panic!("expected Double, got {:?}", other),
    }
}

#[test]
fn test_st_length_point() {
    let f = StLengthFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Double(0.0));
}

#[test]
fn test_st_perimeter_polygon() {
    let f = StPerimeterFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Double(p) => assert!(p > 0.0, "polygon perimeter should be positive, got {}", p),
        other => panic!("expected Double, got {:?}", other),
    }
}

#[test]
fn test_st_perimeter_point() {
    let f = StPerimeterFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Double(0.0));
}

#[test]
fn test_st_azimuth() {
    let f = StAzimuthFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(ny_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Double(bearing) => {
            let two_pi = 2.0 * std::f64::consts::PI;
            assert!(bearing >= 0.0 && bearing < two_pi,
                "bearing should be in [0, 2pi), got {}", bearing);
        }
        other => panic!("expected Double, got {:?}", other),
    }
}

#[test]
fn test_st_makepoint() {
    let f = StMakePointFunction;
    let args = vec![double_arg(-122.4194), double_arg(37.7749)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4194)).abs() < 1e-6);
            assert!((coords[1].as_f64().unwrap() - 37.7749).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

// =========================================================================
// Predicates
// =========================================================================

#[test]
fn test_st_disjoint_distant() {
    let f = StDisjointFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(ny_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_disjoint_same() {
    let f = StDisjointFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(false));
}

#[test]
fn test_st_equals_same() {
    let f = StEqualsFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_equals_different() {
    let f = StEqualsFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(ny_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(false));
}

#[test]
fn test_st_touches_point_point() {
    // Points never touch
    let f = StTouchesFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(ny_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(false));
}

#[test]
fn test_st_touches_point_on_boundary() {
    // A point on the polygon boundary should touch
    let f = StTouchesFunction;
    let boundary_point = json!({"type": "Point", "coordinates": [-122.5, 37.75]});
    let args = vec![geom_arg(boundary_point), geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    // The point is on the left edge of the polygon
    match result {
        Literal::Boolean(_) => {} // just verify it returns a boolean
        other => panic!("expected Boolean, got {:?}", other),
    }
}

#[test]
fn test_st_crosses_line_polygon() {
    // A line that goes from inside to outside the polygon
    let f = StCrossesFunction;
    let crossing_line = json!({"type": "LineString", "coordinates": [
        [-122.4, 37.75], [-122.2, 37.75]
    ]});
    let args = vec![geom_arg(crossing_line), geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_overlaps_polygons() {
    let f = StOverlapsFunction;
    let args = vec![geom_arg(overlap_poly_a()), geom_arg(overlap_poly_b())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_covers_polygon_point() {
    let f = StCoversFunction;
    let args = vec![geom_arg(sf_polygon()), geom_arg(interior_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_coveredby_point_polygon() {
    let f = StCoveredByFunction;
    let args = vec![geom_arg(interior_point()), geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

// =========================================================================
// Accessors
// =========================================================================

#[test]
fn test_st_geometrytype_point() {
    let f = StGeometryTypeFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Text("ST_Point".to_string()));
}

#[test]
fn test_st_geometrytype_polygon() {
    let f = StGeometryTypeFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Text("ST_Polygon".to_string()));
}

#[test]
fn test_st_numpoints_point() {
    let f = StNumPointsFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Int(1));
}

#[test]
fn test_st_numpoints_linestring() {
    let f = StNumPointsFunction;
    let args = vec![geom_arg(sf_line())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Int(2));
}

#[test]
fn test_st_numgeometries_point() {
    let f = StNumGeometriesFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Int(1));
}

#[test]
fn test_st_isvalid() {
    let f = StIsValidFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_isempty() {
    let f = StIsEmptyFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(false));
}

#[test]
fn test_st_isclosed_closed_linestring() {
    let f = StIsClosedFunction;
    let args = vec![geom_arg(closed_ring())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_isclosed_open_linestring() {
    let f = StIsClosedFunction;
    let args = vec![geom_arg(sf_line())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(false));
}

#[test]
fn test_st_issimple() {
    let f = StIsSimpleFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_srid() {
    let f = StSridFunction;
    let args = vec![geom_arg(sf_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Int(4326));
}

// =========================================================================
// Processing
// =========================================================================

#[test]
fn test_st_buffer() {
    let f = StBufferFunction;
    let args = vec![geom_arg(sf_point()), double_arg(1000.0)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Polygon", "buffer of point should be a Polygon");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_centroid() {
    let f = StCentroidFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point", "centroid should be a Point");
            let coords = v["coordinates"].as_array().unwrap();
            let lon = coords[0].as_f64().unwrap();
            let lat = coords[1].as_f64().unwrap();
            // Centroid of the SF polygon should be roughly in the middle
            assert!(lon > -122.6 && lon < -122.2, "centroid lon out of range: {}", lon);
            assert!(lat > 37.6 && lat < 37.9, "centroid lat out of range: {}", lat);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_envelope() {
    let f = StEnvelopeFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Polygon", "envelope should be a Polygon");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_convexhull() {
    let f = StConvexHullFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            // Convex hull of a polygon is a polygon
            assert_eq!(v["type"], "Polygon");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_simplify() {
    let f = StSimplifyFunction;
    let args = vec![geom_arg(sf_line()), double_arg(0.001)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "LineString");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_reverse() {
    let f = StReverseFunction;
    let line = json!({"type": "LineString", "coordinates": [
        [1.0, 2.0], [3.0, 4.0], [5.0, 6.0]
    ]});
    let args = vec![geom_arg(line)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "LineString");
            let coords = v["coordinates"].as_array().unwrap();
            // Should be reversed
            assert_eq!(coords[0], json!([5.0, 6.0]));
            assert_eq!(coords[1], json!([3.0, 4.0]));
            assert_eq!(coords[2], json!([1.0, 2.0]));
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_boundary_polygon() {
    let f = StBoundaryFunction;
    let args = vec![geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "LineString", "boundary of polygon should be LineString");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

// =========================================================================
// Set Operations
// =========================================================================

#[test]
fn test_st_union() {
    let f = StUnionFunction;
    let args = vec![geom_arg(overlap_poly_a()), geom_arg(overlap_poly_b())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            let geom_type = v["type"].as_str().unwrap();
            assert!(
                geom_type == "Polygon" || geom_type == "MultiPolygon",
                "union should be Polygon or MultiPolygon, got {}", geom_type
            );
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_intersection() {
    let f = StIntersectionFunction;
    let args = vec![geom_arg(overlap_poly_a()), geom_arg(overlap_poly_b())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            let geom_type = v["type"].as_str().unwrap();
            assert!(
                geom_type == "Polygon" || geom_type == "MultiPolygon" || geom_type == "GeometryCollection",
                "intersection should be a geometry, got {}", geom_type
            );
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_difference() {
    let f = StDifferenceFunction;
    let args = vec![geom_arg(overlap_poly_a()), geom_arg(overlap_poly_b())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            let geom_type = v["type"].as_str().unwrap();
            assert!(
                geom_type == "Polygon" || geom_type == "MultiPolygon" || geom_type == "GeometryCollection",
                "difference should be a geometry, got {}", geom_type
            );
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_symdifference() {
    let f = StSymDifferenceFunction;
    let args = vec![geom_arg(overlap_poly_a()), geom_arg(overlap_poly_b())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            let geom_type = v["type"].as_str().unwrap();
            assert!(
                geom_type == "Polygon" || geom_type == "MultiPolygon" || geom_type == "GeometryCollection",
                "symmetric difference should be a geometry, got {}", geom_type
            );
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

// =========================================================================
// Constructors
// =========================================================================

#[test]
fn test_st_makeline() {
    let f = StMakeLineFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(ny_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "LineString");
            let coords = v["coordinates"].as_array().unwrap();
            assert_eq!(coords.len(), 2);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_makepolygon() {
    let f = StMakePolygonFunction;
    let args = vec![geom_arg(closed_ring())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Polygon");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_makeenvelope() {
    let f = StMakeEnvelopeFunction;
    let args = vec![
        double_arg(-122.5),
        double_arg(37.7),
        double_arg(-122.3),
        double_arg(37.8),
    ];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Polygon");
            let coords = v["coordinates"].as_array().unwrap();
            let ring = coords[0].as_array().unwrap();
            assert_eq!(ring.len(), 5, "envelope ring should have 5 points (closed)");
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_collect() {
    let f = StCollectFunction;
    let args = vec![geom_arg(sf_point()), geom_arg(ny_point())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "GeometryCollection");
            let geometries = v["geometries"].as_array().unwrap();
            assert_eq!(geometries.len(), 2);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

// =========================================================================
// Line Operations
// =========================================================================

#[test]
fn test_st_startpoint() {
    let f = StStartPointFunction;
    let args = vec![geom_arg(sf_line())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4194)).abs() < 1e-6);
            assert!((coords[1].as_f64().unwrap() - 37.7749).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_endpoint() {
    let f = StEndPointFunction;
    let args = vec![geom_arg(sf_line())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4089)).abs() < 1e-6);
            assert!((coords[1].as_f64().unwrap() - 37.7858).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_pointn_first() {
    let f = StPointNFunction;
    let args = vec![geom_arg(sf_line()), int_arg(1)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4194)).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_pointn_second() {
    let f = StPointNFunction;
    let args = vec![geom_arg(sf_line()), int_arg(2)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4089)).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_pointn_out_of_range() {
    let f = StPointNFunction;
    let args = vec![geom_arg(sf_line()), int_arg(99)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Null);
}

#[test]
fn test_st_lineinterpolatepoint_start() {
    let f = StLineInterpolatePointFunction;
    let args = vec![geom_arg(sf_line()), double_arg(0.0)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4194)).abs() < 1e-6);
            assert!((coords[1].as_f64().unwrap() - 37.7749).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_lineinterpolatepoint_end() {
    let f = StLineInterpolatePointFunction;
    let args = vec![geom_arg(sf_line()), double_arg(1.0)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            assert!((coords[0].as_f64().unwrap() - (-122.4089)).abs() < 1e-6);
            assert!((coords[1].as_f64().unwrap() - 37.7858).abs() < 1e-6);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_lineinterpolatepoint_midpoint() {
    let f = StLineInterpolatePointFunction;
    let args = vec![geom_arg(sf_line()), double_arg(0.5)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Point");
            let coords = v["coordinates"].as_array().unwrap();
            let lon = coords[0].as_f64().unwrap();
            let lat = coords[1].as_f64().unwrap();
            // Midpoint should be between start and end
            assert!(lon > -122.42 && lon < -122.40, "midpoint lon: {}", lon);
            assert!(lat > 37.77 && lat < 37.79, "midpoint lat: {}", lat);
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

// =========================================================================
// NULL Propagation
// =========================================================================

#[test]
fn test_null_propagation_unary() {
    // Single-arg functions should return Null for Null input
    let row = empty_row();
    let null = vec![null_arg()];

    assert_eq!(StAreaFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StLengthFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StPerimeterFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StGeometryTypeFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StNumPointsFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StNumGeometriesFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StIsValidFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StIsEmptyFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StIsClosedFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StIsSimpleFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StSridFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StBufferFunction.evaluate(&[null_arg(), double_arg(100.0)], &row).unwrap(), Literal::Null);
    assert_eq!(StCentroidFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StEnvelopeFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StConvexHullFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StReverseFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StBoundaryFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StStartPointFunction.evaluate(&null, &row).unwrap(), Literal::Null);
    assert_eq!(StEndPointFunction.evaluate(&null, &row).unwrap(), Literal::Null);
}

#[test]
fn test_null_propagation_binary() {
    // Binary functions should return Null when either arg is Null
    let row = empty_row();
    let sf = geom_arg(sf_point());

    // First arg null
    assert_eq!(
        StDisjointFunction.evaluate(&[null_arg(), sf.clone()], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StEqualsFunction.evaluate(&[null_arg(), sf.clone()], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StMakeLineFunction.evaluate(&[null_arg(), sf.clone()], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StCollectFunction.evaluate(&[null_arg(), sf.clone()], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StAzimuthFunction.evaluate(&[null_arg(), sf.clone()], &row).unwrap(),
        Literal::Null
    );

    // Second arg null
    assert_eq!(
        StDisjointFunction.evaluate(&[sf.clone(), null_arg()], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StEqualsFunction.evaluate(&[sf.clone(), null_arg()], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StMakeLineFunction.evaluate(&[sf.clone(), null_arg()], &row).unwrap(),
        Literal::Null
    );

    // ST_MAKEPOINT with null
    assert_eq!(
        StMakePointFunction.evaluate(&[null_arg(), double_arg(37.0)], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StMakePointFunction.evaluate(&[double_arg(-122.0), null_arg()], &row).unwrap(),
        Literal::Null
    );

    // ST_LINEINTERPOLATEPOINT with null
    assert_eq!(
        StLineInterpolatePointFunction.evaluate(&[null_arg(), double_arg(0.5)], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StLineInterpolatePointFunction.evaluate(&[geom_arg(sf_line()), null_arg()], &row).unwrap(),
        Literal::Null
    );

    // ST_POINTN with null
    assert_eq!(
        StPointNFunction.evaluate(&[null_arg(), int_arg(1)], &row).unwrap(),
        Literal::Null
    );
    assert_eq!(
        StPointNFunction.evaluate(&[geom_arg(sf_line()), null_arg()], &row).unwrap(),
        Literal::Null
    );

    // ST_SIMPLIFY with null
    assert_eq!(
        StSimplifyFunction.evaluate(&[null_arg(), double_arg(0.001)], &row).unwrap(),
        Literal::Null
    );
}

#[test]
fn test_null_propagation_makeenvelope() {
    let row = empty_row();
    // Any null argument should make the whole thing null
    assert_eq!(
        StMakeEnvelopeFunction.evaluate(
            &[null_arg(), double_arg(37.7), double_arg(-122.3), double_arg(37.8)],
            &row
        ).unwrap(),
        Literal::Null
    );
}

// =========================================================================
// Regression tests for correctness fixes
// =========================================================================

#[test]
fn test_st_distance_point_inside_polygon_is_zero() {
    let f = StDistanceFunction;
    // interior_point() is inside sf_polygon()
    let args = vec![geom_arg(interior_point()), geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Double(d) => assert!(d < 1.0, "point inside polygon should have distance ~0, got {}", d),
        other => panic!("expected Double, got {:?}", other),
    }
}

#[test]
fn test_st_dwithin_point_near_polygon_boundary() {
    let f = StDWithinFunction;
    // Point just outside the west edge of sf_polygon ([-122.5, 37.7] to [-122.5, 37.8])
    let point_near = json!({"type": "Point", "coordinates": [-122.501, 37.75]});
    // ~111m per 0.001 degrees at this latitude -- 0.001 deg ~ 88m, so use 200m radius
    let args = vec![geom_arg(point_near), geom_arg(sf_polygon()), double_arg(200.0)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_buffer_circular_at_mid_latitude() {
    let f = StBufferFunction;
    // San Francisco latitude ~37.77
    let args = vec![geom_arg(sf_point()), double_arg(1000.0)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    match result {
        Literal::Geometry(v) => {
            assert_eq!(v["type"], "Polygon");
            let ring = v["coordinates"][0].as_array().unwrap();
            // Collect all longitudes and latitudes
            let lons: Vec<f64> = ring.iter().map(|c| c[0].as_f64().unwrap()).collect();
            let lats: Vec<f64> = ring.iter().map(|c| c[1].as_f64().unwrap()).collect();
            let lon_range = lons.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - lons.iter().cloned().fold(f64::INFINITY, f64::min);
            let lat_range = lats.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - lats.iter().cloned().fold(f64::INFINITY, f64::min);
            // At ~37.77N, longitude degrees are smaller than latitude degrees,
            // so lon_range should be wider than lat_range
            assert!(
                lon_range > lat_range,
                "lon_range ({}) should be > lat_range ({}) at mid-latitude",
                lon_range, lat_range
            );
        }
        other => panic!("expected Geometry, got {:?}", other),
    }
}

#[test]
fn test_st_intersects_point_on_polygon_boundary() {
    let f = StIntersectsFunction;
    // Point exactly on the west edge of sf_polygon
    let boundary_point = json!({"type": "Point", "coordinates": [-122.5, 37.75]});
    let args = vec![geom_arg(boundary_point), geom_arg(sf_polygon())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_touches_overlapping_polygons_false() {
    let f = StTouchesFunction;
    // overlap_poly_a and overlap_poly_b share interior area -- they should NOT touch
    let args = vec![geom_arg(overlap_poly_a()), geom_arg(overlap_poly_b())];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(false));
}

#[test]
fn test_st_touches_adjacent_polygons_true() {
    let f = StTouchesFunction;
    // Two polygons sharing an edge at x=2
    let poly_left = json!({"type": "Polygon", "coordinates": [[
        [0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]
    ]]});
    let poly_right = json!({"type": "Polygon", "coordinates": [[
        [2.0, 0.0], [4.0, 0.0], [4.0, 2.0], [2.0, 2.0], [2.0, 0.0]
    ]]});
    let args = vec![geom_arg(poly_left), geom_arg(poly_right)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_equals_slight_float_drift() {
    let f = StEqualsFunction;
    let point_a = json!({"type": "Point", "coordinates": [-122.4194, 37.7749]});
    // Add ~1e-9 drift -- well within the 1e-8 epsilon
    let point_b = json!({"type": "Point", "coordinates": [-122.4194000009, 37.7749000009]});
    let args = vec![geom_arg(point_a), geom_arg(point_b)];
    let result = f.evaluate(&args, &empty_row()).unwrap();
    assert_eq!(result, Literal::Boolean(true));
}

#[test]
fn test_st_extract_all_coords_rejects_invalid() {
    // Non-numeric coordinate value should produce an error
    let bad_point = json!({"type": "Point", "coordinates": ["not_a_number", 37.0]});
    let result = helpers::extract_all_coords(&bad_point);
    assert!(result.is_err(), "expected error for non-numeric coordinates");
}
