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

//! Tests for PropertyValue and related types.

use super::*;
use chrono::{TimeZone, Utc};
use std::collections::HashMap;

#[test]
fn test_datetime_timestamp_json_serialization() {
    let timestamp = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let dt: DateTimeTimestamp = timestamp.into();

    // Serialize to JSON as RFC3339 string
    let json = serde_json::to_value(&dt).expect("should serialize");
    assert!(json.as_str().unwrap().contains("2023-11-14"));

    // Deserialize from JSON RFC3339 string
    let deserialized: DateTimeTimestamp = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(deserialized, dt);
}

#[test]
fn test_datetime_timestamp_rejects_integers_in_json() {
    // JSON deserialization should reject integers to avoid ambiguity
    // with PropertyValue's Number variant (untagged enum)
    let json = serde_json::json!(1_700_000_000);
    let result: Result<DateTimeTimestamp, _> = serde_json::from_value(json);
    assert!(result.is_err(), "Should reject integer in JSON");
}

#[test]
fn test_property_value_date_json_serialization() {
    let timestamp = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let value = PropertyValue::Date(timestamp.into());

    // Serialize to JSON as RFC3339 string
    let json = serde_json::to_value(&value).expect("should serialize");
    assert!(json.as_str().unwrap().contains("2023-11-14"));

    // Deserialize from JSON RFC3339 string
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(deserialized, value);
}

#[test]
fn test_property_value_integer_becomes_integer_not_date() {
    // Integers should deserialize as Integer, not Date
    // This is the key fix: JSON numbers are unambiguously Integer
    let json = serde_json::json!(1_700_000_000);
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    // Should be Integer, not Date
    assert!(
        matches!(deserialized, PropertyValue::Integer(_)),
        "Integer should deserialize as Integer, not Date"
    );

    if let PropertyValue::Integer(n) = deserialized {
        assert_eq!(n, 1_700_000_000);
    }
}

#[test]
fn test_property_value_small_integer_becomes_integer() {
    // Small integers like 8 should also be Integer
    let json = serde_json::json!(8);
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    assert!(
        matches!(deserialized, PropertyValue::Integer(_)),
        "Small integer should deserialize as Integer"
    );

    if let PropertyValue::Integer(n) = deserialized {
        assert_eq!(n, 8);
    }
}

#[test]
fn test_property_value_float() {
    // Numbers with decimal points should be Float
    let json = serde_json::json!(3.14159);
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    assert!(
        matches!(deserialized, PropertyValue::Float(_)),
        "Decimal number should deserialize as Float"
    );

    if let PropertyValue::Float(n) = deserialized {
        assert!((n - 3.14159).abs() < 0.0001);
    }
}

#[test]
fn test_property_value_null() {
    let json = serde_json::json!(null);
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    assert!(
        matches!(deserialized, PropertyValue::Null),
        "null should deserialize as Null"
    );
}

#[test]
fn test_property_value_large_integer_precision() {
    // Large integers should preserve precision (unlike f64)
    let large_int = 9_007_199_254_740_993_i64; // Beyond f64 safe integer range
    let json = serde_json::json!(large_int);
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    if let PropertyValue::Integer(n) = deserialized {
        assert_eq!(n, large_int, "Large integer should preserve exact value");
    } else {
        panic!("Expected Integer variant");
    }
}

#[test]
fn test_raisin_url_minimal() {
    let json = serde_json::json!({
        "raisin:url": "https://example.com"
    });
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    if let PropertyValue::Url(url) = deserialized {
        assert_eq!(url.url, "https://example.com");
        assert!(url.title.is_none());
        assert!(url.image.is_none());
    } else {
        panic!("Expected Url variant");
    }
}

#[test]
fn test_raisin_url_rich() {
    let json = serde_json::json!({
        "raisin:url": "https://blog.example.com/post/123",
        "raisin:title": "How to Build a Database",
        "raisin:description": "A comprehensive guide...",
        "raisin:image": "https://blog.example.com/og-image.jpg",
        "raisin:type": "article"
    });
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    if let PropertyValue::Url(url) = deserialized {
        assert_eq!(url.url, "https://blog.example.com/post/123");
        assert_eq!(url.title, Some("How to Build a Database".to_string()));
        assert_eq!(
            url.description,
            Some("A comprehensive guide...".to_string())
        );
        assert_eq!(
            url.image,
            Some("https://blog.example.com/og-image.jpg".to_string())
        );
        assert_eq!(url.link_type, Some("article".to_string()));
    } else {
        panic!("Expected Url variant");
    }
}

#[test]
fn test_raisin_url_video_embed() {
    let json = serde_json::json!({
        "raisin:url": "https://youtube.com/watch?v=abc123",
        "raisin:type": "video",
        "raisin:embed": "https://youtube.com/embed/abc123",
        "raisin:duration": 342,
        "raisin:width": 1920,
        "raisin:height": 1080
    });
    let deserialized: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    if let PropertyValue::Url(url) = deserialized {
        assert_eq!(url.url, "https://youtube.com/watch?v=abc123");
        assert_eq!(url.link_type, Some("video".to_string()));
        assert_eq!(
            url.embed_url,
            Some("https://youtube.com/embed/abc123".to_string())
        );
        assert_eq!(url.duration, Some(342));
        assert_eq!(url.width, Some(1920));
        assert_eq!(url.height, Some(1080));
    } else {
        panic!("Expected Url variant");
    }
}

#[test]
fn test_raisin_url_builder() {
    let url = RaisinUrl::parse("https://example.com")
        .expect("valid url")
        .with_title("Example Site")
        .with_description("An example website")
        .external();

    assert_eq!(url.url, "https://example.com/");
    assert_eq!(url.title, Some("Example Site".to_string()));
    assert_eq!(url.description, Some("An example website".to_string()));
    assert_eq!(url.target, Some("_blank".to_string()));
    assert_eq!(url.rel, Some("noopener".to_string()));
}

#[test]
fn test_resource_messagepack_serialization() {
    let timestamp1 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let timestamp2 = Utc.timestamp_opt(1_700_000_500, 0).unwrap();

    let resource = Resource {
        uuid: "test-uuid".to_string(),
        name: Some("test.txt".to_string()),
        size: Some(1024),
        mime_type: Some("text/plain".to_string()),
        url: Some("http://example.com/test.txt".to_string()),
        metadata: None,
        is_loaded: Some(true),
        is_external: Some(false),
        created_at: timestamp1.into(),
        updated_at: timestamp2.into(),
    };

    // Serialize with MessagePack
    let bytes = rmp_serde::to_vec(&resource).expect("should serialize");

    // Deserialize with MessagePack
    let deserialized: Resource = rmp_serde::from_slice(&bytes).expect("should deserialize");

    assert_eq!(deserialized.uuid, "test-uuid");
    assert_eq!(deserialized.created_at.timestamp(), 1_700_000_000);
    assert_eq!(deserialized.updated_at.timestamp(), 1_700_000_500);
}

#[test]
fn test_property_value_date_messagepack_serialization() {
    let timestamp = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let value = PropertyValue::Date(timestamp.into());

    // Serialize with MessagePack
    let bytes = rmp_serde::to_vec(&value).expect("should serialize");

    // Deserialize with MessagePack
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    assert_eq!(deserialized, value);
}

// === GeoJSON Tests ===

#[test]
fn test_geojson_point_serialization() {
    let point = GeoJson::point(-122.4194, 37.7749);

    // Serialize to JSON
    let json = serde_json::to_value(&point).expect("should serialize");
    assert_eq!(json["type"], "Point");
    assert_eq!(json["coordinates"][0], -122.4194);
    assert_eq!(json["coordinates"][1], 37.7749);

    // Deserialize from JSON
    let deserialized: GeoJson = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(deserialized, point);
}

#[test]
fn test_geojson_point_from_json() {
    let json = serde_json::json!({
        "type": "Point",
        "coordinates": [-122.4194, 37.7749]
    });

    let point: GeoJson = serde_json::from_value(json).expect("should deserialize");

    match point {
        GeoJson::Point { coordinates } => {
            assert_eq!(coordinates[0], -122.4194);
            assert_eq!(coordinates[1], 37.7749);
        }
        _ => panic!("Expected Point"),
    }
}

#[test]
fn test_geojson_polygon_from_json() {
    let json = serde_json::json!({
        "type": "Polygon",
        "coordinates": [[
            [-122.5, 37.7],
            [-122.3, 37.7],
            [-122.3, 37.8],
            [-122.5, 37.8],
            [-122.5, 37.7]
        ]]
    });

    let polygon: GeoJson = serde_json::from_value(json).expect("should deserialize");

    match polygon {
        GeoJson::Polygon { coordinates } => {
            assert_eq!(coordinates.len(), 1); // One ring
            assert_eq!(coordinates[0].len(), 5); // 5 points (closed)
        }
        _ => panic!("Expected Polygon"),
    }
}

#[test]
fn test_property_value_geometry_from_json() {
    let json = serde_json::json!({
        "type": "Point",
        "coordinates": [-122.4194, 37.7749]
    });

    let value: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    match value {
        PropertyValue::Geometry(GeoJson::Point { coordinates }) => {
            assert_eq!(coordinates[0], -122.4194);
            assert_eq!(coordinates[1], 37.7749);
        }
        _ => panic!("Expected Geometry(Point), got {:?}", value),
    }
}

#[test]
fn test_geojson_centroid() {
    let point = GeoJson::point(-122.4194, 37.7749);
    assert_eq!(point.centroid(), Some([-122.4194, 37.7749]));

    let line = GeoJson::LineString {
        coordinates: vec![[-122.0, 37.0], [-123.0, 38.0]],
    };
    let centroid = line.centroid().unwrap();
    assert!((centroid[0] - (-122.5)).abs() < 0.001);
    assert!((centroid[1] - 37.5).abs() < 0.001);
}

#[test]
fn test_geojson_is_point() {
    let point = GeoJson::point(-122.4194, 37.7749);
    assert!(point.is_point());

    let line = GeoJson::LineString {
        coordinates: vec![[-122.0, 37.0], [-123.0, 38.0]],
    };
    assert!(!line.is_point());
}

#[test]
fn test_geojson_messagepack_serialization() {
    let point = GeoJson::point(-122.4194, 37.7749);

    // Serialize with MessagePack
    let bytes = rmp_serde::to_vec(&point).expect("should serialize");

    // Deserialize with MessagePack
    let deserialized: GeoJson = rmp_serde::from_slice(&bytes).expect("should deserialize");

    assert_eq!(deserialized, point);
}

#[test]
fn test_property_value_geometry_messagepack() {
    let value = PropertyValue::Geometry(GeoJson::point(-122.4194, 37.7749));

    // Serialize with MessagePack
    let bytes = rmp_serde::to_vec(&value).expect("should serialize");

    // Deserialize with MessagePack
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    assert_eq!(deserialized, value);
}

#[test]
fn test_raisin_reference_json_serialization() {
    let reference = RaisinReference {
        id: "test-uuid".to_string(),
        workspace: "social".to_string(),
        path: "/news-feed-demo/tags/tech-stack/rust".to_string(),
    };

    // Serialize RaisinReference directly - should have raisin:* keys
    let json = serde_json::to_string(&reference).expect("should serialize");
    assert!(json.contains("\"raisin:ref\""));
    assert!(json.contains("\"raisin:workspace\""));
    assert!(json.contains("\"raisin:path\""));

    // Test PropertyValue::Reference serialization (untagged enum passes through)
    let pv = PropertyValue::Reference(reference.clone());
    let pv_json = serde_json::to_string(&pv).expect("should serialize");
    assert!(pv_json.contains("\"raisin:ref\""));

    // Test Array of references - should be array of objects, not array of arrays
    let arr = PropertyValue::Array(vec![
        PropertyValue::Reference(reference.clone()),
        PropertyValue::Reference(RaisinReference {
            id: "uuid2".to_string(),
            workspace: "social".to_string(),
            path: "/other/path".to_string(),
        }),
    ]);
    let arr_json = serde_json::to_string(&arr).expect("should serialize");
    assert!(arr_json.contains("\"raisin:ref\""));
    assert!(!arr_json.starts_with("[[")); // Not array of arrays
}

#[test]
fn test_raisin_reference_messagepack_roundtrip() {
    // MessagePack serializes structs as arrays by field order, not as maps.
    // The deserializer must handle both formats.
    let reference = RaisinReference {
        id: "test-uuid".to_string(),
        workspace: "social".to_string(),
        path: "/news-feed-demo/tags/tech-stack/rust".to_string(),
    };

    let pv = PropertyValue::Reference(reference.clone());
    let bytes = rmp_serde::to_vec(&pv).expect("should serialize to msgpack");
    let deserialized: PropertyValue =
        rmp_serde::from_slice(&bytes).expect("should deserialize from msgpack");

    // Verify roundtrip preserved the Reference type and values
    if let PropertyValue::Reference(r) = &deserialized {
        assert_eq!(r.id, "test-uuid");
        assert_eq!(r.workspace, "social");
        assert_eq!(r.path, "/news-feed-demo/tags/tech-stack/rust");
    } else {
        panic!("Expected PropertyValue::Reference, got {:?}", deserialized);
    }

    // JSON serialization after roundtrip should have proper object format
    let json = serde_json::to_string(&deserialized).expect("should serialize to json");
    assert!(json.contains("\"raisin:ref\""));
    assert!(json.contains("\"raisin:workspace\""));
}

#[test]
fn test_two_element_string_array_not_deserialized_as_reference() {
    // Plain string arrays like keywords should NOT be deserialized as RaisinReference
    // This was a bug: ["test", "integration"] was incorrectly matched as a reference tuple
    let keywords = PropertyValue::Array(vec![
        PropertyValue::String("test".to_string()),
        PropertyValue::String("integration".to_string()),
    ]);

    // Serialize to MessagePack
    let bytes = rmp_serde::to_vec(&keywords).expect("should serialize");

    // Deserialize - should remain Array, not become Reference
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    match deserialized {
        PropertyValue::Array(arr) => {
            assert_eq!(arr.len(), 2);
            assert!(
                matches!(&arr[0], PropertyValue::String(s) if s == "test"),
                "First element should be 'test', got {:?}",
                arr[0]
            );
            assert!(
                matches!(&arr[1], PropertyValue::String(s) if s == "integration"),
                "Second element should be 'integration', got {:?}",
                arr[1]
            );
        }
        other => panic!("Expected Array, got {:?}", other),
    }
}

#[test]
fn test_three_element_string_array_not_deserialized_as_reference() {
    // Three element string array should also remain Array (not be confused with [id, workspace, path])
    let keywords = PropertyValue::Array(vec![
        PropertyValue::String("rust".to_string()),
        PropertyValue::String("database".to_string()),
        PropertyValue::String("hierarchical".to_string()),
    ]);

    let bytes = rmp_serde::to_vec(&keywords).expect("should serialize");
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    assert!(
        matches!(deserialized, PropertyValue::Array(_)),
        "Three-element string array should remain Array, got {:?}",
        deserialized
    );
}

#[test]
fn test_real_reference_with_uuid_still_works() {
    // A real reference with UUID-like id should still be recognized
    let reference = RaisinReference {
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(), // UUID with hyphens
        workspace: "content".to_string(),
        path: "/articles/my-post".to_string(),
    };

    let pv = PropertyValue::Reference(reference);
    let bytes = rmp_serde::to_vec(&pv).expect("should serialize");
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    if let PropertyValue::Reference(r) = deserialized {
        assert_eq!(r.id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(r.workspace, "content");
    } else {
        panic!("Expected Reference, got {:?}", deserialized);
    }
}

#[test]
fn test_real_reference_with_nanoid_still_works() {
    // A real reference with nanoid-like id (21+ chars) should still be recognized
    let reference = RaisinReference {
        id: "V1StGXR8_Z5jdHi6B-myT".to_string(), // 21 char nanoid
        workspace: "content".to_string(),
        path: "".to_string(),
    };

    let pv = PropertyValue::Reference(reference);
    let bytes = rmp_serde::to_vec(&pv).expect("should serialize");
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    if let PropertyValue::Reference(r) = deserialized {
        assert_eq!(r.id, "V1StGXR8_Z5jdHi6B-myT");
        assert_eq!(r.workspace, "content");
    } else {
        panic!("Expected Reference, got {:?}", deserialized);
    }
}

#[test]
fn test_real_reference_with_path_id_still_works() {
    // A reference where id is a path should still be recognized
    let reference = RaisinReference {
        id: "/articles/my-post".to_string(), // Path-based id
        workspace: "content".to_string(),
        path: "/articles/my-post".to_string(),
    };

    let pv = PropertyValue::Reference(reference);
    let bytes = rmp_serde::to_vec(&pv).expect("should serialize");
    let deserialized: PropertyValue = rmp_serde::from_slice(&bytes).expect("should deserialize");

    if let PropertyValue::Reference(r) = deserialized {
        assert_eq!(r.id, "/articles/my-post");
        assert_eq!(r.workspace, "content");
    } else {
        panic!("Expected Reference, got {:?}", deserialized);
    }
}

// === Element Flat Serialization Tests ===

#[test]
fn test_element_json_flat_serialization() {
    // Create an element with content
    let mut content = HashMap::new();
    content.insert(
        "title".to_string(),
        PropertyValue::String("My Task".to_string()),
    );
    content.insert(
        "note".to_string(),
        PropertyValue::String("Important".to_string()),
    );
    content.insert("priority".to_string(), PropertyValue::Integer(1));

    let element = Element {
        uuid: "el-123".to_string(),
        element_type: "launchpad:KanbanCard".to_string(),
        content,
    };

    // Serialize to JSON
    let json = serde_json::to_value(&element).expect("should serialize");

    // Should be flat (no "content" wrapper)
    assert_eq!(json["element_type"], "launchpad:KanbanCard");
    assert_eq!(json["uuid"], "el-123");
    assert_eq!(json["title"], "My Task");
    assert_eq!(json["note"], "Important");
    assert_eq!(json["priority"], 1);

    // Should not have a "content" field
    assert!(
        json.get("content").is_none(),
        "Should not have content wrapper"
    );
}

#[test]
fn test_element_json_flat_deserialization() {
    // Flat JSON format
    let json = serde_json::json!({
        "element_type": "launchpad:KanbanCard",
        "uuid": "el-123",
        "title": "My Task",
        "note": "Important",
        "priority": 1
    });

    // Deserialize
    let element: Element = serde_json::from_value(json).expect("should deserialize");

    // Check structure
    assert_eq!(element.uuid, "el-123");
    assert_eq!(element.element_type, "launchpad:KanbanCard");
    assert_eq!(element.content.len(), 3);

    // Check content fields
    assert_eq!(
        element.content.get("title"),
        Some(&PropertyValue::String("My Task".to_string()))
    );
    assert_eq!(
        element.content.get("note"),
        Some(&PropertyValue::String("Important".to_string()))
    );
    assert_eq!(
        element.content.get("priority"),
        Some(&PropertyValue::Integer(1))
    );
}

#[test]
fn test_element_json_roundtrip() {
    // Create element
    let mut content = HashMap::new();
    content.insert(
        "title".to_string(),
        PropertyValue::String("Original".to_string()),
    );
    content.insert("count".to_string(), PropertyValue::Integer(42));

    let original = Element {
        uuid: "el-456".to_string(),
        element_type: "test:Element".to_string(),
        content,
    };

    // Serialize to JSON
    let json = serde_json::to_value(&original).expect("should serialize");

    // Deserialize back
    let roundtrip: Element = serde_json::from_value(json).expect("should deserialize");

    // Should match original
    assert_eq!(roundtrip, original);
}

#[test]
fn test_element_without_uuid_serialization() {
    // Create element without uuid (empty string)
    let mut content = HashMap::new();
    content.insert(
        "name".to_string(),
        PropertyValue::String("Test".to_string()),
    );

    let element = Element {
        uuid: String::new(),
        element_type: "test:Type".to_string(),
        content,
    };

    // Serialize to JSON
    let json = serde_json::to_value(&element).expect("should serialize");

    // Should not include uuid field
    assert!(json.get("uuid").is_none(), "Empty uuid should be skipped");
    assert_eq!(json["element_type"], "test:Type");
    assert_eq!(json["name"], "Test");
}

#[test]
fn test_element_without_uuid_deserialization() {
    // Flat JSON without uuid field
    let json = serde_json::json!({
        "element_type": "test:Type",
        "name": "Test"
    });

    // Deserialize
    let element: Element = serde_json::from_value(json).expect("should deserialize");

    // uuid should be empty string
    assert_eq!(element.uuid, "");
    assert_eq!(element.element_type, "test:Type");
    assert_eq!(element.content.len(), 1);
}

#[test]
fn test_element_messagepack_roundtrip() {
    // Create element
    let mut content = HashMap::new();
    content.insert(
        "field1".to_string(),
        PropertyValue::String("value1".to_string()),
    );
    content.insert("field2".to_string(), PropertyValue::Integer(100));
    content.insert("field3".to_string(), PropertyValue::Boolean(true));

    let original = Element {
        uuid: "el-msgpack".to_string(),
        element_type: "test:MsgPack".to_string(),
        content,
    };

    // Serialize to MessagePack
    let bytes = rmp_serde::to_vec(&original).expect("should serialize to msgpack");

    // Deserialize from MessagePack
    let roundtrip: Element =
        rmp_serde::from_slice(&bytes).expect("should deserialize from msgpack");

    // Should match original
    assert_eq!(roundtrip, original);
}

#[test]
fn test_element_complex_content_types() {
    // Test with various PropertyValue types in content
    let mut content = HashMap::new();
    content.insert(
        "string".to_string(),
        PropertyValue::String("text".to_string()),
    );
    content.insert("integer".to_string(), PropertyValue::Integer(42));
    content.insert("float".to_string(), PropertyValue::Float(3.14));
    content.insert("boolean".to_string(), PropertyValue::Boolean(true));
    content.insert("null".to_string(), PropertyValue::Null);
    // Use heterogeneous array (String and Integer) to avoid Vector deserialization
    content.insert(
        "array".to_string(),
        PropertyValue::Array(vec![
            PropertyValue::String("one".to_string()),
            PropertyValue::Integer(2),
        ]),
    );

    let element = Element {
        uuid: "el-complex".to_string(),
        element_type: "test:Complex".to_string(),
        content,
    };

    // JSON roundtrip
    let json = serde_json::to_value(&element).expect("should serialize");
    let from_json: Element = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(from_json, element);

    // MessagePack roundtrip
    let bytes = rmp_serde::to_vec(&element).expect("should serialize to msgpack");
    let from_msgpack: Element =
        rmp_serde::from_slice(&bytes).expect("should deserialize from msgpack");
    assert_eq!(from_msgpack, element);
}

#[test]
fn test_element_missing_element_type_error() {
    // JSON without element_type should fail
    let json = serde_json::json!({
        "uuid": "el-bad",
        "title": "Missing Type"
    });

    let result: Result<Element, _> = serde_json::from_value(json);
    assert!(result.is_err(), "Should error on missing element_type");
    assert!(result.unwrap_err().to_string().contains("element_type"));
}

#[test]
fn test_element_property_value_wrapper() {
    // Test Element within PropertyValue enum
    let mut content = HashMap::new();
    content.insert(
        "data".to_string(),
        PropertyValue::String("test".to_string()),
    );

    let element = Element {
        uuid: "el-pv".to_string(),
        element_type: "test:Type".to_string(),
        content,
    };

    let pv = PropertyValue::Element(element.clone());

    // JSON roundtrip
    let json = serde_json::to_value(&pv).expect("should serialize");
    let from_json: PropertyValue = serde_json::from_value(json).expect("should deserialize");

    if let PropertyValue::Element(e) = from_json {
        assert_eq!(e, element);
    } else {
        panic!("Expected PropertyValue::Element");
    }
}

#[test]
fn test_element_empty_content() {
    // Element with no content fields
    let element = Element {
        uuid: "el-empty".to_string(),
        element_type: "test:Empty".to_string(),
        content: HashMap::new(),
    };

    // JSON roundtrip
    let json = serde_json::to_value(&element).expect("should serialize");
    assert_eq!(json["element_type"], "test:Empty");
    assert_eq!(json["uuid"], "el-empty");

    let from_json: Element = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(from_json, element);
    assert_eq!(from_json.content.len(), 0);
}

#[test]
fn test_element_nested_objects() {
    // Test Element with nested objects in content
    let mut inner_map = HashMap::new();
    inner_map.insert(
        "nested_field".to_string(),
        PropertyValue::String("nested_value".to_string()),
    );

    let mut content = HashMap::new();
    content.insert(
        "title".to_string(),
        PropertyValue::String("Parent".to_string()),
    );
    content.insert("metadata".to_string(), PropertyValue::Object(inner_map));

    let element = Element {
        uuid: "el-nested".to_string(),
        element_type: "test:Nested".to_string(),
        content,
    };

    // JSON roundtrip
    let json = serde_json::to_value(&element).expect("should serialize");
    let from_json: Element = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(from_json, element);
}

#[test]
fn test_element_json_output_format() {
    // Verify the actual JSON format produced
    let mut content = HashMap::new();
    content.insert(
        "title".to_string(),
        PropertyValue::String("My Task".to_string()),
    );
    content.insert("priority".to_string(), PropertyValue::Integer(1));

    let element = Element {
        uuid: "el-123".to_string(),
        element_type: "launchpad:KanbanCard".to_string(),
        content,
    };

    let json = serde_json::to_value(&element).expect("should serialize");

    // Verify it's flat (fields at top level, no content wrapper)
    assert!(json.is_object());
    let obj = json.as_object().unwrap();

    // Should have element_type and uuid at top level
    assert!(obj.contains_key("element_type"));
    assert!(obj.contains_key("uuid"));

    // Should have content fields at top level
    assert!(obj.contains_key("title"));
    assert!(obj.contains_key("priority"));

    // Should NOT have a content wrapper
    assert!(!obj.contains_key("content"));

    // Print for manual verification
    eprintln!(
        "Flat JSON format:
{}",
        serde_json::to_string_pretty(&element).unwrap()
    );
}

#[test]
fn test_element_backward_compat_nested_content() {
    // Old format with nested "content" wrapper - should be unwrapped during deserialization
    let old_format = serde_json::json!({
        "element_type": "launchpad:Hero",
        "uuid": "hero-1",
        "content": {
            "headline": "Welcome",
            "subheadline": "Start here"
        }
    });

    let element: Element =
        serde_json::from_value(old_format).expect("should deserialize old format");

    // Should unwrap the nested content
    assert_eq!(element.element_type, "launchpad:Hero");
    assert_eq!(element.uuid, "hero-1");
    assert_eq!(element.content.len(), 2);
    assert_eq!(
        element.content.get("headline"),
        Some(&PropertyValue::String("Welcome".to_string()))
    );
    assert_eq!(
        element.content.get("subheadline"),
        Some(&PropertyValue::String("Start here".to_string()))
    );

    // When re-serialized, should be flat (no content wrapper)
    let json = serde_json::to_value(&element).expect("should serialize");
    let obj = json.as_object().unwrap();
    assert!(obj.contains_key("headline"));
    assert!(obj.contains_key("subheadline"));
    assert!(!obj.contains_key("content"));
}

#[test]
fn test_element_field_named_content() {
    // Edge case: element with an actual field named "content" alongside other fields
    // This should preserve "content" as a regular field
    let json = serde_json::json!({
        "element_type": "launchpad:TextBlock",
        "uuid": "text-1",
        "heading": "My Heading",
        "content": "This is the text content"
    });

    let element: Element = serde_json::from_value(json).expect("should deserialize");

    assert_eq!(element.element_type, "launchpad:TextBlock");
    assert_eq!(element.content.len(), 2);
    assert_eq!(
        element.content.get("heading"),
        Some(&PropertyValue::String("My Heading".to_string()))
    );
    assert_eq!(
        element.content.get("content"),
        Some(&PropertyValue::String(
            "This is the text content".to_string()
        ))
    );
}
