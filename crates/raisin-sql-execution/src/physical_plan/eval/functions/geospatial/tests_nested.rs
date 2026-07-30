//! Row-level nested geospatial: addressing, and the row semantics of a node that
//! carries several geometries.
//!
//! This is the FALLBACK path — what runs before a rebuild has drained, and what a
//! wildcard path always takes. Its correctness is the difference between "an
//! unindexed nested query is slow" and "an unindexed nested query silently
//! returns nothing".

use super::st_distance::StDistanceFunction;
use super::st_dwithin::StDWithinFunction;
use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use std::collections::HashMap;

/// `properties->>'<path>'` — the ONE spelling nested geometry is addressed by.
fn path_arg(path: &str) -> TypedExpr {
    TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(TypedExpr::new(
                Expr::Column {
                    table: "nodes".into(),
                    column: "properties".into(),
                },
                DataType::JsonB,
            )),
            key: Box::new(TypedExpr::new(
                Expr::Literal(Literal::Text(path.to_string())),
                DataType::Text,
            )),
        },
        DataType::Text,
    )
}

fn center(lon: f64, lat: f64) -> TypedExpr {
    TypedExpr::new(
        Expr::Literal(Literal::Geometry(
            serde_json::to_value(GeoJson::point(lon, lat)).unwrap(),
        )),
        DataType::Geometry,
    )
}

fn radius(m: f64) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Double(m)), DataType::Double)
}

fn point(lon: f64, lat: f64) -> PropertyValue {
    PropertyValue::Geometry(GeoJson::point(lon, lat))
}

fn element(field: &str, value: PropertyValue) -> PropertyValue {
    PropertyValue::Element(Element {
        uuid: String::new(),
        element_type: "demo:Section".into(),
        content: HashMap::from([(field.to_string(), value)]),
    })
}

/// The owner's motivating node: THREE geometries at three different depths.
///
/// * `location`      — top level, Zurich HB           (8.5402, 47.3779)
/// * `venue.geo`     — inside an object, ~1.5 km away (8.5600, 47.3779)
/// * `stops.N.geo`   — an array of elements, spreading east
fn three_geometry_row() -> Row {
    let properties = HashMap::from([
        ("location".to_string(), point(8.5402, 47.3779)),
        (
            "venue".to_string(),
            PropertyValue::Object(HashMap::from([("geo".to_string(), point(8.5600, 47.3779))])),
        ),
        (
            "stops".to_string(),
            PropertyValue::Array(vec![
                element("geo", point(8.7000, 47.3779)),
                element("geo", point(8.5500, 47.3779)),
                element("geo", point(8.9000, 47.3779)),
            ]),
        ),
    ]);
    let mut row = Row::new();
    row.insert("nodes.properties".into(), PropertyValue::Object(properties));
    row
}

fn dwithin(row: &Row, path: &str, meters: f64) -> Literal {
    StDWithinFunction
        .evaluate(
            &[path_arg(path), center(8.5402, 47.3779), radius(meters)],
            row,
        )
        .unwrap()
}

fn distance(row: &Row, path: &str) -> f64 {
    match StDistanceFunction
        .evaluate(&[path_arg(path), center(8.5402, 47.3779)], row)
        .unwrap()
    {
        Literal::Double(d) => d,
        other => panic!("expected a distance, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Q12 — addressing: naming WHICH field is searched
// ---------------------------------------------------------------------------

#[test]
fn a_top_level_path_is_addressed_by_its_bare_name() {
    let row = three_geometry_row();
    assert_eq!(distance(&row, "location").round(), 0.0);
    assert_eq!(dwithin(&row, "location", 10.0), Literal::Boolean(true));
}

#[test]
fn an_object_nested_path_is_addressed_by_its_dotted_path() {
    let row = three_geometry_row();
    // ~1.49 km east.
    let d = distance(&row, "venue.geo");
    assert!((1400.0..1600.0).contains(&d), "distance was {d}");
    assert_eq!(dwithin(&row, "venue.geo", 2000.0), Literal::Boolean(true));
    assert_eq!(dwithin(&row, "venue.geo", 1000.0), Literal::Boolean(false));
}

#[test]
fn one_element_of_an_array_is_addressed_by_its_concrete_index() {
    let row = three_geometry_row();
    // stops.1 is the nearest of the three, ~740 m.
    let d = distance(&row, "stops.1.geo");
    assert!((600.0..900.0).contains(&d), "distance was {d}");
    // stops.0 is ~12 km, well outside.
    assert_eq!(
        dwithin(&row, "stops.0.geo", 2000.0),
        Literal::Boolean(false)
    );
}

/// Naming one field searches ONLY that field. This is the whole point of the
/// per-field index namespace: a node whose `location` is 0 m away must not match
/// a query about `venue.geo` at 10 m.
#[test]
fn naming_a_field_searches_only_that_field() {
    let row = three_geometry_row();
    assert_eq!(dwithin(&row, "location", 10.0), Literal::Boolean(true));
    assert_eq!(dwithin(&row, "venue.geo", 10.0), Literal::Boolean(false));
    assert_eq!(dwithin(&row, "stops.0.geo", 10.0), Literal::Boolean(false));
}

/// An addressable-but-absent path is NULL, not an error and not a false match.
#[test]
fn a_path_that_does_not_exist_yields_null() {
    let row = three_geometry_row();
    assert_eq!(dwithin(&row, "nowhere.geo", 10.0), Literal::Null);
    assert_eq!(
        StDistanceFunction
            .evaluate(
                &[path_arg("nowhere.geo"), center(8.5402, 47.3779)],
                &three_geometry_row()
            )
            .unwrap(),
        Literal::Null
    );
}

// ---------------------------------------------------------------------------
// Q13 — row semantics when one node matches via several geometries
// ---------------------------------------------------------------------------

/// A wildcard is TRUE when ANY matched geometry is within the radius.
#[test]
fn a_wildcard_matches_when_any_element_is_within_the_radius() {
    let row = three_geometry_row();
    // Only stops.1 (~740 m) is within 1 km; stops.0 and stops.2 are far.
    assert_eq!(dwithin(&row, "stops[].geo", 1000.0), Literal::Boolean(true));
    // Nothing is within 100 m.
    assert_eq!(dwithin(&row, "stops[].geo", 100.0), Literal::Boolean(false));
}

/// A wildcard distance is the MINIMUM over the matched geometries — "how close
/// does this node get". Minimum and not first-found, because it is the only
/// choice that makes `ORDER BY ... LIMIT k` mean "the k nearest nodes".
#[test]
fn a_wildcard_distance_is_the_minimum_over_the_matched_geometries() {
    let row = three_geometry_row();
    let min = distance(&row, "stops[].geo");
    let nearest = distance(&row, "stops.1.geo");
    assert!((min - nearest).abs() < 1e-6, "{min} vs {nearest}");
    // And strictly less than the others, so "first found" would have differed.
    assert!(min < distance(&row, "stops.0.geo"));
    assert!(min < distance(&row, "stops.2.geo"));
}

/// A wildcard over an EMPTY array is NULL, not zero and not false-with-a-distance.
#[test]
fn a_wildcard_over_an_empty_array_yields_null() {
    let mut row = Row::new();
    row.insert(
        "nodes.properties".into(),
        PropertyValue::Object(HashMap::from([(
            "stops".to_string(),
            PropertyValue::Array(vec![]),
        )])),
    );
    assert_eq!(dwithin(&row, "stops[].geo", 1000.0), Literal::Null);
}

/// A wildcard has no single value, so a function with no defined answer over a
/// set must say so rather than silently pick an element. `ST_INTERSECTS` on a
/// silently-picked element would be a wrong answer, not a slow one.
#[test]
fn a_wildcard_is_rejected_by_functions_with_no_set_semantics() {
    use super::st_intersects::StIntersectsFunction;
    let row = three_geometry_row();
    let err = StIntersectsFunction
        .evaluate(&[path_arg("stops[].geo"), center(8.5402, 47.3779)], &row)
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("wildcard"), "{message}");
    assert!(message.contains("ST_DWITHIN"), "{message}");
}

// ---------------------------------------------------------------------------
// Ambiguity rule: a direct key wins over a path walk
// ---------------------------------------------------------------------------

/// A property whose NAME contains a dot is read directly; the path walk is only
/// a fallback. Documented, deliberately not disambiguated — the same limitation
/// the reference index has always carried.
#[test]
fn a_literal_dotted_property_name_wins_over_the_path_walk() {
    let mut row = Row::new();
    row.insert(
        "nodes.properties".into(),
        PropertyValue::Object(HashMap::from([
            // Literally named "venue.geo", at the query centre.
            ("venue.geo".to_string(), point(8.5402, 47.3779)),
            // And a real nested venue.geo, far away.
            (
                "venue".to_string(),
                PropertyValue::Object(HashMap::from([("geo".to_string(), point(9.5, 47.3779))])),
            ),
        ])),
    );
    assert_eq!(dwithin(&row, "venue.geo", 10.0), Literal::Boolean(true));
}
