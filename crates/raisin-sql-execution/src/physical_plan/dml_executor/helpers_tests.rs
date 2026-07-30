// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Unit cover for the DML value converter's geometry handling.
//!
//! # The regression these pin
//!
//! `json_value_to_property_value` is a hand-rolled mirror of `PropertyValue`'s
//! canonical `#[serde(untagged)]` ladder, and it was missing the `Geometry` arm
//! that sits at slot 9 of that ladder — ahead of `Object` at slot 11. Every SQL
//! write path funnels through it, so a geometry written with `INSERT` or `UPDATE`
//! arrived at the low-level write functions as `PropertyValue::Object`. Automatic
//! type-driven spatial indexing keys off `PropertyValue::Geometry`, so the index
//! entry was never written and `ST_DWITHIN` returned nothing — with no error
//! anywhere, because the node itself stored and read back perfectly (reading the
//! stored blob DOES go through the canonical ladder, which yields `Geometry`).
//!
//! The end-to-end proof lives in
//! `raisin-server/tests/all/spatial_index_lifecycle_test.rs`; these are the cheap
//! guards that fail in seconds rather than minutes.

use super::helpers::{json_value_to_property_value, literal_to_property_value};
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_sql::analyzer::Literal;
use serde_json::json;

#[test]
fn a_geojson_object_becomes_geometry_not_object() {
    let value = json!({ "type": "Point", "coordinates": [8.5402, 47.3782] });
    match json_value_to_property_value(&value).expect("conversion") {
        PropertyValue::Geometry(GeoJson::Point { coordinates, .. }) => {
            assert_eq!(coordinates.x, 8.5402);
            assert_eq!(coordinates.y, 47.3782);
            assert_eq!(coordinates.z, None);
        }
        other => panic!("a GeoJSON Point must convert to Geometry, got {other:?}"),
    }
}

#[test]
fn altitude_and_srid_survive_the_converter() {
    let value = json!({
        "type": "Point",
        "coordinates": [8.5402, 47.3782, 100.0],
        "srid": 4326
    });
    match json_value_to_property_value(&value).expect("conversion") {
        PropertyValue::Geometry(geometry) => {
            assert_eq!(geometry.z_range(), Some((100.0, 100.0)));
        }
        other => panic!("expected Geometry, got {other:?}"),
    }
}

#[test]
fn every_geometry_type_converts() {
    // Delegating to `GeoJson`'s own deserializer rather than sniffing for a `type`
    // key is what makes this list free — including the Multi* and collection types
    // a hand-rolled check would have had to enumerate.
    for value in [
        json!({ "type": "Point", "coordinates": [0.0, 0.0] }),
        json!({ "type": "LineString", "coordinates": [[0.0, 0.0], [1.0, 1.0]] }),
        json!({ "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]] }),
        json!({ "type": "MultiPoint", "coordinates": [[0.0, 0.0], [1.0, 1.0]] }),
        json!({ "type": "MultiLineString",
                "coordinates": [[[0.0, 0.0], [1.0, 1.0]]] }),
        json!({ "type": "MultiPolygon",
                "coordinates": [[[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]] }),
        json!({ "type": "GeometryCollection", "geometries": [] }),
    ] {
        assert!(
            matches!(
                json_value_to_property_value(&value),
                Ok(PropertyValue::Geometry(_))
            ),
            "must convert to Geometry: {value}"
        );
    }
}

#[test]
fn a_malformed_geometry_still_falls_through_to_object() {
    // Same behaviour as the canonical ladder: not-well-formed GeoJSON is an
    // ordinary object. It is NOT silently discarded, and it is NOT an error here —
    // rejecting it belongs at the schema-validation layer, where the NodeType's
    // declared `PropertyType::Geometry` is in scope.
    let value = json!({ "type": "Point", "coordinates": "nonsense" });
    assert!(matches!(
        json_value_to_property_value(&value),
        Ok(PropertyValue::Object(_))
    ));

    // A plain object that merely has a `type` key is untouched.
    let value = json!({ "type": "invoice", "total": 12 });
    assert!(matches!(
        json_value_to_property_value(&value),
        Ok(PropertyValue::Object(_))
    ));
}

#[test]
fn a_geometry_valued_expression_can_be_assigned_to_a_property() {
    // `SET location = ST_POINT(8.54, 47.37)` evaluates to `Literal::Geometry`,
    // which used to hit the catch-all and fail the whole statement with
    // "Cannot convert literal".
    let literal = Literal::Geometry(json!({ "type": "Point", "coordinates": [8.54, 47.37] }));
    assert!(matches!(
        literal_to_property_value(&literal),
        Ok(PropertyValue::Geometry(_))
    ));
}

#[test]
fn a_nested_geometry_inside_the_properties_object_converts() {
    // The shape every real INSERT uses: the whole `properties` column as JSONB.
    let literal = Literal::JsonB(json!({
        "title": "kiosk",
        "floor": "L2",
        "location": { "type": "Point", "coordinates": [8.5402, 47.3782] }
    }));
    let PropertyValue::Object(map) = literal_to_property_value(&literal).expect("conversion")
    else {
        panic!("the properties column must convert to an object");
    };
    assert!(
        matches!(map.get("location"), Some(PropertyValue::Geometry(_))),
        "the nested geometry must be Geometry, or the spatial index hook never fires"
    );
    assert!(matches!(map.get("floor"), Some(PropertyValue::String(_))));
}
