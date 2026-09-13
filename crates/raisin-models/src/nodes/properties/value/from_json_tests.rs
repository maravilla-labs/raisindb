// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Round-trip guards for [`PropertyValue::from_json`].
//!
//! The property that matters: a value that was STORED as a domain type and then
//! travels through JSON (a SQL `properties || $1::jsonb` merge, a function that
//! reads a node and writes it back, a WS client echoing a node) must come back as
//! the same variant, or the declared-type check refuses a write that never
//! touched it.

use super::super::*;
use chrono::Utc;
use serde_json::json;

fn round_trip(original: &PropertyValue) -> PropertyValue {
    PropertyValue::from_json(&serde_json::to_value(original).expect("serialize"))
}

fn resource() -> PropertyValue {
    let now = Utc::now();
    PropertyValue::Resource(Resource {
        uuid: "u-1".into(),
        name: Some("a.mp3".into()),
        size: Some(42),
        mime_type: Some("audio/mpeg".into()),
        url: None,
        metadata: None,
        is_loaded: Some(true),
        is_external: Some(false),
        created_at: now.into(),
        updated_at: now.into(),
    })
}

fn element(kind: &str) -> Element {
    let mut content = std::collections::HashMap::new();
    content.insert("title".to_string(), PropertyValue::String("Hi".into()));
    Element {
        uuid: format!("{kind}-1"),
        element_type: kind.into(),
        content,
    }
}

#[test]
fn a_stored_resource_survives_json() {
    let original = resource();
    assert_eq!(round_trip(&original), original);
}

#[test]
fn a_resource_nested_in_an_object_survives_json() {
    let mut bag = std::collections::HashMap::new();
    bag.insert("file".to_string(), resource());
    bag.insert("title".to_string(), PropertyValue::String("t".into()));
    let original = PropertyValue::Object(bag);
    assert_eq!(round_trip(&original), original);
}

#[test]
fn every_domain_object_survives_json() {
    let reference = PropertyValue::Reference(RaisinReference {
        id: "n-1".into(),
        workspace: "stories".into(),
        path: "/a/b".into(),
    });
    let url = PropertyValue::Url(RaisinUrl::new("https://example.com/x"));
    let el = PropertyValue::Element(element("hero"));
    let composite = PropertyValue::Composite(Composite {
        uuid: "c-1".into(),
        items: vec![element("text"), element("image")],
    });
    let geometry: PropertyValue =
        PropertyValue::from_json(&json!({ "type": "Point", "coordinates": [8.5, 47.3] }));

    for original in [reference, url, el, composite] {
        assert_eq!(round_trip(&original), original);
    }
    assert!(matches!(geometry, PropertyValue::Geometry(_)));
}

#[test]
fn strings_are_never_reinterpreted() {
    // The canonical untagged ladder turns these into Decimal / Date. A write path
    // must not: only a DECLARED type may coerce a string.
    for s in ["10", "19.90", "1e3", "2026-09-09T10:00:00Z", "hello"] {
        assert_eq!(
            PropertyValue::from_json(&json!(s)),
            PropertyValue::String(s.into())
        );
    }
}

#[test]
fn arrays_stay_arrays_and_nulls_stay_null() {
    assert_eq!(
        PropertyValue::from_json(&json!([])),
        PropertyValue::Array(vec![])
    );
    assert_eq!(
        PropertyValue::from_json(&json!([1, 2.5])),
        PropertyValue::Array(vec![PropertyValue::Integer(1), PropertyValue::Float(2.5)])
    );
    assert_eq!(PropertyValue::from_json(&json!(null)), PropertyValue::Null);
}

#[test]
fn a_reference_may_omit_its_workspace() {
    match PropertyValue::from_json(&json!({ "raisin:ref": "/x" })) {
        PropertyValue::Reference(r) => {
            assert_eq!(r.id, "/x");
            assert!(r.workspace.is_empty());
        }
        other => panic!("expected Reference, got {other:?}"),
    }
}

#[test]
fn look_alikes_stay_plain_objects() {
    // Has uuid + items but also another field: a bag, not a Composite.
    let bag = json!({ "uuid": "x", "items": [], "title": "keep me" });
    assert!(matches!(PropertyValue::from_json(&bag), PropertyValue::Object(m) if m.len() == 3));

    // Has uuid + created_at but is not a well-formed Resource.
    let bag = json!({ "uuid": "x", "created_at": "not a date" });
    assert!(matches!(
        PropertyValue::from_json(&bag),
        PropertyValue::Object(_)
    ));

    // Has a `type` key but is not GeoJSON.
    let bag = json!({ "type": "ticket", "seats": 2 });
    assert!(matches!(
        PropertyValue::from_json(&bag),
        PropertyValue::Object(_)
    ));
}

#[test]
fn legacy_nested_element_content_is_unwrapped_like_the_canonical_reader() {
    let v = json!({ "element_type": "text", "uuid": "e", "content": { "body": "hi" } });
    match PropertyValue::from_json(&v) {
        PropertyValue::Element(e) => {
            assert_eq!(
                e.content.get("body"),
                Some(&PropertyValue::String("hi".into()))
            );
            assert!(!e.content.contains_key("content"));
        }
        other => panic!("expected Element, got {other:?}"),
    }
    // A text block whose FIELD is named content stays a field.
    let v = json!({ "element_type": "text", "uuid": "e", "content": "hello" });
    match PropertyValue::from_json(&v) {
        PropertyValue::Element(e) => {
            assert_eq!(
                e.content.get("content"),
                Some(&PropertyValue::String("hello".into()))
            );
        }
        other => panic!("expected Element, got {other:?}"),
    }
}
