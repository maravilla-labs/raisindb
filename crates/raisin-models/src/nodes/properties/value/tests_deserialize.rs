// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The hand-written `PropertyValue` deserializer must decide exactly what the
//! derived untagged ladder decided. `Ladder` below IS that derive, kept
//! verbatim as the oracle; every case is decoded both ways and compared, over
//! MessagePack (what storage holds) and JSON.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;
use crate::nodes::properties::utils::{
    deserialize_guarded_string, deserialize_raisin_reference, deserialize_raisin_url,
    deserialize_tagged_decimal,
};
use crate::timestamp::StorageTimestamp;

/// The derived ladder `PropertyValue` used before, variant for variant.
#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum Ladder {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Date(StorageTimestamp),
    #[serde(deserialize_with = "deserialize_tagged_decimal")]
    Decimal(Decimal),
    #[serde(deserialize_with = "deserialize_guarded_string")]
    String(String),
    #[serde(deserialize_with = "deserialize_raisin_reference")]
    Reference(RaisinReference),
    #[serde(deserialize_with = "deserialize_raisin_url")]
    Url(RaisinUrl),
    Resource(Resource),
    Composite(Composite),
    Element(Element),
    Vector(Vec<f32>),
    Geometry(GeoJson),
    Array(Vec<Ladder>),
    Object(HashMap<String, Ladder>),
}

impl From<Ladder> for PropertyValue {
    fn from(l: Ladder) -> Self {
        match l {
            Ladder::Null => PropertyValue::Null,
            Ladder::Boolean(v) => PropertyValue::Boolean(v),
            Ladder::Integer(v) => PropertyValue::Integer(v),
            Ladder::Float(v) => PropertyValue::Float(v),
            Ladder::Date(v) => PropertyValue::Date(v),
            Ladder::Decimal(v) => PropertyValue::Decimal(v),
            Ladder::String(v) => PropertyValue::String(v),
            Ladder::Reference(v) => PropertyValue::Reference(v),
            Ladder::Url(v) => PropertyValue::Url(v),
            Ladder::Resource(v) => PropertyValue::Resource(v),
            Ladder::Composite(v) => PropertyValue::Composite(v),
            Ladder::Element(v) => PropertyValue::Element(v),
            Ladder::Vector(v) => PropertyValue::Vector(v),
            Ladder::Geometry(v) => PropertyValue::Geometry(v),
            Ladder::Array(v) => PropertyValue::Array(v.into_iter().map(Into::into).collect()),
            Ladder::Object(v) => {
                PropertyValue::Object(v.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
        }
    }
}

fn from_msgpack(bytes: &[u8]) -> (Option<PropertyValue>, Option<PropertyValue>) {
    let new = rmp_serde::from_slice::<PropertyValue>(bytes).ok();
    let old = rmp_serde::from_slice::<Ladder>(bytes).ok().map(Into::into);
    (new, old)
}

fn from_json(value: &serde_json::Value) -> (Option<PropertyValue>, Option<PropertyValue>) {
    let new = serde_json::from_value::<PropertyValue>(value.clone()).ok();
    let old = serde_json::from_value::<Ladder>(value.clone())
        .ok()
        .map(Into::into);
    (new, old)
}

/// Decode `value` from MessagePack (both struct encodings) and from JSON, and
/// require the two deserializers to agree every time.
fn assert_same<T: Serialize + std::fmt::Debug>(value: &T) {
    for bytes in [
        rmp_serde::to_vec(value).unwrap(),
        rmp_serde::to_vec_named(value).unwrap(),
    ] {
        let (new, old) = from_msgpack(&bytes);
        assert!(new.is_some(), "msgpack does not decode: {value:?}");
        assert_eq!(new, old, "msgpack decode differs for {value:?}");
    }
    let json = serde_json::to_value(value).unwrap();
    let (new, old) = from_json(&json);
    assert!(new.is_some(), "json does not decode: {json}");
    assert_eq!(new, old, "json decode differs for {json}");
}

fn resource() -> Resource {
    Resource {
        uuid: "r1".into(),
        name: Some("a.jpg".into()),
        size: Some(12),
        mime_type: Some("image/jpeg".into()),
        url: None,
        metadata: Some(HashMap::from([(
            "exif".to_string(),
            PropertyValue::Object(HashMap::from([(
                "iso".to_string(),
                PropertyValue::Integer(200),
            )])),
        )])),
        is_loaded: Some(true),
        is_external: None,
        created_at: StorageTimestamp::from_nanos(1_700_000_000_000_000_000).unwrap(),
        updated_at: StorageTimestamp::from_nanos(1_700_000_000_000_000_001).unwrap(),
    }
}

fn element(kind: &str, content: Vec<(&str, PropertyValue)>) -> Element {
    Element {
        uuid: format!("{kind}-1"),
        element_type: kind.into(),
        content: content
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn typed_values() -> Vec<PropertyValue> {
    let s = |v: &str| PropertyValue::String(v.into());
    let reference = PropertyValue::Reference(RaisinReference {
        id: "abc".into(),
        workspace: "assets".into(),
        path: "/a.jpg".into(),
    });
    let block = element(
        "bap:Teaser",
        vec![
            ("headline", s("Hello")),
            ("image", reference.clone()),
            (
                "cta",
                PropertyValue::Object(HashMap::from([
                    ("label".to_string(), s("More")),
                    ("style".to_string(), PropertyValue::Object(HashMap::new())),
                ])),
            ),
            ("tags", PropertyValue::Array(vec![s("a"), s("b")])),
        ],
    );
    vec![
        PropertyValue::Null,
        PropertyValue::Boolean(true),
        PropertyValue::Integer(-7),
        PropertyValue::Integer(i64::MAX),
        PropertyValue::Float(1.5),
        PropertyValue::Float(2.0),
        PropertyValue::Date(StorageTimestamp::from_nanos(1_750_000_000_123_456_789).unwrap()),
        PropertyValue::Decimal("19.90".parse().unwrap()),
        PropertyValue::Decimal("5".parse().unwrap()),
        s("hello"),
        s("05"),
        s("76133"),
        s("19.90"),
        s("1e3"),
        s("2024-05-01T10:00:00Z"),
        s("2024-05-01"),
        s(""),
        reference.clone(),
        PropertyValue::Url(RaisinUrl::new("https://example.com")),
        PropertyValue::Resource(resource()),
        PropertyValue::Composite(Composite {
            uuid: "c1".into(),
            items: vec![block.clone()],
        }),
        PropertyValue::Element(block.clone()),
        PropertyValue::Vector(vec![0.25, -1.0, 3.0]),
        PropertyValue::Vector(vec![]),
        PropertyValue::Geometry(
            serde_json::from_value(json!({"type": "Point", "coordinates": [8.1, 48.7]})).unwrap(),
        ),
        PropertyValue::Array(vec![PropertyValue::Integer(3)]),
        PropertyValue::Array(vec![s("x"), PropertyValue::Integer(1), PropertyValue::Null]),
        PropertyValue::Array(vec![
            PropertyValue::Element(block.clone()),
            reference.clone(),
        ]),
        PropertyValue::Object(HashMap::from([
            (
                "content".to_string(),
                PropertyValue::Array(vec![PropertyValue::Element(block)]),
            ),
            ("hero".to_string(), reference),
            ("uuid".to_string(), s("not-a-resource")),
            ("type".to_string(), s("not-a-geometry")),
        ])),
    ]
}

#[test]
fn typed_values_decode_like_the_derived_ladder() {
    for value in typed_values() {
        assert_same(&value);
        // And nested one and two levels down, where the old ladder compounded.
        let nested = PropertyValue::Object(HashMap::from([("v".to_string(), value.clone())]));
        assert_same(&nested);
        assert_same(&PropertyValue::Array(vec![nested]));
    }
}

/// Shapes nothing in `PropertyValue` serializes to, but that JSON input or
/// older blobs can carry: they must fall on the same side of every gate.
#[test]
fn raw_shapes_decode_like_the_derived_ladder() {
    let cases = vec![
        json!(null),
        json!(u64::MAX),
        json!([3]),
        json!([1_750_000_000_000_000_000_i64]),
        json!([1, 2.5, 3]),
        json!([]),
        json!(["Point", [1.0, 2.0]]),
        json!(["a", "b", "c"]),
        json!({}),
        json!({"raisin:decimal": "1.50"}),
        json!({"raisin:decimal": "1.50", "x": 1}),
        json!({"raisin:string": "76133"}),
        json!({"raisin:ref": "id"}),
        json!({"raisin:ref": "id", "raisin:workspace": "ws"}),
        json!({"raisin:ref": "id", "raisin:workspace": "ws", "extra": {"a": 1}}),
        json!({"raisin:url": "https://x.y", "title": "X"}),
        json!({"raisin:url": 5}),
        json!({"uuid": "u", "created_at": "2024-01-01T00:00:00Z", "updated_at": "2024-01-01T00:00:00Z"}),
        json!({"uuid": "u", "created_at": "nope", "updated_at": "2024-01-01T00:00:00Z"}),
        json!({"uuid": "u", "items": []}),
        json!({"uuid": "u", "items": [{"element_type": "x:Y", "a": 1}]}),
        json!({"uuid": "u", "items": "no"}),
        json!({"element_type": "x:Y", "uuid": "e", "content": {"a": {"b": [1, {"c": null}]}}}),
        json!({"element_type": "x:Y", "content": 3}),
        json!({"element_type": 7}),
        json!({"type": "Point", "coordinates": [1.0, 2.0]}),
        json!({"type": "Point", "coordinates": "x"}),
        json!({"type": "Feature"}),
        json!({"a": {"b": {"c": {"d": {"e": {"element_type": "x:Deep", "f": [1, "2", {"g": true}]}}}}}}),
    ];
    for case in cases {
        let (new, old) = from_json(&case);
        assert_eq!(new, old, "json decode differs for {case}");
        let bytes = rmp_serde::to_vec(&case).unwrap();
        let (new, old) = from_msgpack(&bytes);
        assert_eq!(new, old, "msgpack decode differs for {case}");
    }
}

/// A deterministic pseudo-random walk over nested shapes, so the gates are
/// exercised in combinations nobody wrote down.
#[test]
fn generated_trees_decode_like_the_derived_ladder() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }
    const KEYS: [&str; 10] = [
        "element_type",
        "uuid",
        "items",
        "type",
        "raisin:ref",
        "raisin:workspace",
        "created_at",
        "title",
        "content",
        "raisin:url",
    ];
    fn gen(rng: &mut Rng, depth: u32) -> serde_json::Value {
        let leaf = depth == 0 || rng.next(3) == 0;
        match if leaf { rng.next(6) } else { 6 + rng.next(2) } {
            0 => json!(null),
            1 => json!(rng.next(2) == 1),
            2 => json!(rng.next(1000) as i64 - 500),
            3 => json!(rng.next(1000) as f64 / 8.0),
            4 => json!(
                ["x:Y", "Point", "2024-01-01T00:00:00Z", "05", "12.5", "id"][rng.next(6) as usize]
            ),
            5 => json!([]),
            6 => (0..rng.next(4)).map(|_| gen(rng, depth - 1)).collect(),
            _ => {
                let mut map = serde_json::Map::new();
                for _ in 0..rng.next(5) {
                    let key = KEYS[rng.next(KEYS.len() as u64) as usize];
                    map.insert(key.into(), gen(rng, depth - 1));
                }
                serde_json::Value::Object(map)
            }
        }
    }
    let mut rng = Rng(42);
    for _ in 0..4000 {
        let case = gen(&mut rng, 5);
        let (new, old) = from_json(&case);
        assert_eq!(new, old, "json decode differs for {case}");
        let (new, old) = from_msgpack(&rmp_serde::to_vec(&case).unwrap());
        assert_eq!(new, old, "msgpack decode differs for {case}");
    }
}
