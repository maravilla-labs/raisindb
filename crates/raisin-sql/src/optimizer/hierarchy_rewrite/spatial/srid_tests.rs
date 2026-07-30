//! The SRID gate on index-eligible spatial predicates.
//!
//! Companion to [`super::query_center_is_wgs84`]. These assert the *planning*
//! half of the SRID story; the write half (normalising a stored geometry into
//! WGS84 index cells) lives in `raisin_rocksdb::spatial::normalize`.

use super::{extract_distance_order, extract_spatial_predicate};
use crate::analyzer::{DataType, Expr, FunctionCategory, FunctionSignature, Literal, TypedExpr};
use crate::optimizer::hierarchy_rewrite::predicate::CanonicalPredicate;

fn geom_source() -> TypedExpr {
    TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(TypedExpr::column(
                "nodes".into(),
                "properties".into(),
                DataType::JsonB,
            )),
            key: Box::new(TypedExpr::literal(Literal::Text("location".into()))),
        },
        DataType::Text,
    )
}

fn call(name: &str, args: Vec<TypedExpr>, ty: DataType) -> TypedExpr {
    let params = args.iter().map(|a| a.data_type.clone()).collect();
    TypedExpr::new(
        Expr::Function {
            name: name.into(),
            args,
            signature: FunctionSignature {
                name: name.into(),
                params,
                return_type: ty.clone(),
                is_deterministic: true,
                category: FunctionCategory::Scalar,
            },
            filter: None,
        },
        ty,
    )
}

fn point(x: f64, y: f64) -> TypedExpr {
    call(
        "ST_POINT",
        vec![
            TypedExpr::literal(Literal::Double(x)),
            TypedExpr::literal(Literal::Double(y)),
        ],
        DataType::Geometry,
    )
}

fn set_srid(inner: TypedExpr, srid: i32) -> TypedExpr {
    call(
        "ST_SETSRID",
        vec![inner, TypedExpr::literal(Literal::Int(srid))],
        DataType::Geometry,
    )
}

fn dwithin(center: TypedExpr, radius: f64) -> TypedExpr {
    call(
        "ST_DWITHIN",
        vec![
            geom_source(),
            center,
            TypedExpr::literal(Literal::Double(radius)),
        ],
        DataType::Boolean,
    )
}

#[test]
fn a_wgs84_center_is_index_eligible() {
    let p = extract_spatial_predicate(&dwithin(point(8.54, 47.37), 500.0));
    match p {
        Some(CanonicalPredicate::SpatialDWithin {
            center_lon,
            center_lat,
            radius_meters,
            exact,
            ..
        }) => {
            assert_eq!(center_lon, 8.54);
            assert_eq!(center_lat, 47.37);
            assert_eq!(radius_meters, 500.0);
            assert!(
                exact,
                "a point centre in ST_DWITHIN is an exact index match"
            );
        }
        other => panic!("expected an index-eligible SpatialDWithin, got {other:?}"),
    }
}

#[test]
fn an_explicit_4326_label_is_still_index_eligible() {
    let p = extract_spatial_predicate(&dwithin(set_srid(point(8.54, 47.37), 4326), 500.0));
    assert!(
        matches!(p, Some(CanonicalPredicate::SpatialDWithin { .. })),
        "explicit EPSG:4326 must behave exactly like an unlabelled centre"
    );
}

/// The regression this gate exists for. `ST_SETSRID` used to be folded away, so
/// a Web Mercator centre was planned as if 950668 were a longitude — which made
/// the cell planner return `NotCovering` and failed the whole query.
#[test]
fn a_projected_center_declines_the_index_instead_of_being_read_as_degrees() {
    for srid in [3857, 2056, 32632, 31370] {
        let p = extract_spatial_predicate(&dwithin(
            set_srid(point(950_668.45, 6_002_678.0), srid),
            500.0,
        ));
        assert!(
            p.is_none(),
            "EPSG:{srid} centre must fall back to a residual filter, got {p:?}"
        );
    }
}

#[test]
fn a_projected_center_in_a_geojson_literal_also_declines() {
    let literal = TypedExpr::literal(Literal::Geometry(serde_json::json!({
        "type": "Point",
        "coordinates": [950_668.45, 6_002_678.0],
        "srid": 3857,
    })));
    assert!(extract_spatial_predicate(&dwithin(literal, 500.0)).is_none());
}

#[test]
fn the_textual_srid_spellings_are_understood() {
    for form in ["EPSG:3857", "SRID=3857", "3857"] {
        let literal = TypedExpr::literal(Literal::Geometry(serde_json::json!({
            "type": "Point",
            "coordinates": [950_668.45, 6_002_678.0],
            "srid": form,
        })));
        assert!(
            extract_spatial_predicate(&dwithin(literal, 500.0)).is_none(),
            "srid spelled {form} must be recognised as projected"
        );
    }
    // ... and the equivalent 4326 spellings stay eligible.
    for form in ["EPSG:4326", "SRID=4326", "4326"] {
        let literal = TypedExpr::literal(Literal::Geometry(serde_json::json!({
            "type": "Point",
            "coordinates": [8.54, 47.37],
            "srid": form,
        })));
        assert!(
            extract_spatial_predicate(&dwithin(literal, 500.0)).is_some(),
            "srid spelled {form} must stay index-eligible"
        );
    }
}

#[test]
fn an_unparseable_srid_member_declines_rather_than_guessing_wgs84() {
    let literal = TypedExpr::literal(Literal::Geometry(serde_json::json!({
        "type": "Point",
        "coordinates": [8.54, 47.37],
        "srid": {"nonsense": true},
    })));
    assert!(extract_spatial_predicate(&dwithin(literal, 500.0)).is_none());
}

#[test]
fn order_by_distance_to_a_projected_center_also_declines() {
    let order_expr = call(
        "ST_DISTANCE",
        vec![
            geom_source(),
            set_srid(point(950_668.45, 6_002_678.0), 3857),
        ],
        DataType::Double,
    );
    assert!(extract_distance_order(&order_expr).is_none());

    let wgs = call(
        "ST_DISTANCE",
        vec![geom_source(), point(8.54, 47.37)],
        DataType::Double,
    );
    assert!(extract_distance_order(&wgs).is_some());
}
