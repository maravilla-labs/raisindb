use super::super::{Composite, Element, GeoJson, PropertyValue, RaisinReference, RaisinUrl};
use std::collections::HashMap;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn obj(entries: &[(&str, PropertyValue)]) -> PropertyValue {
    PropertyValue::Object(map(entries))
}

fn map(entries: &[(&str, PropertyValue)]) -> HashMap<String, PropertyValue> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn element(uuid: &str, content: &[(&str, PropertyValue)]) -> Element {
    Element {
        uuid: uuid.to_string(),
        element_type: "hero".to_string(),
        content: map(content),
    }
}

/// Equal, or (NaN != NaN) rendered identically.
fn same(a: &PropertyValue, b: &PropertyValue) -> bool {
    a == b || format!("{a:?}") == format!("{b:?}")
}

#[test]
fn equals_the_json_round_trip() {
    let date: PropertyValue =
        serde_json::from_value(serde_json::json!("2026-10-05T12:00:00Z")).unwrap();
    let decimal = PropertyValue::Decimal("19.90".parse().unwrap());
    let reference = PropertyValue::Reference(RaisinReference {
        id: "n1".into(),
        workspace: "ws".into(),
        path: String::new(),
    });
    let reference_with_path = PropertyValue::Reference(RaisinReference {
        id: "n1".into(),
        workspace: String::new(),
        path: "/a".into(),
    });
    let url = PropertyValue::Url(RaisinUrl::new("https://example.com/"));
    let point = PropertyValue::Geometry(GeoJson::Point {
        coordinates: serde_json::from_value(serde_json::json!([1.0, 2.0])).unwrap(),
        srid: None,
    });
    let plain = element("u1", &[("title", s("t")), ("n", PropertyValue::Integer(2))]);
    let mut values = vec![
        PropertyValue::Null,
        PropertyValue::Boolean(false),
        PropertyValue::Integer(-3),
        PropertyValue::Float(2.0),
        PropertyValue::Float(f64::NAN),
        PropertyValue::Float(f64::INFINITY),
        s("2026-10-05T12:00:00Z"),
        s("10"),
        date.clone(),
        decimal.clone(),
        reference.clone(),
        reference_with_path.clone(),
        url.clone(),
        point.clone(),
        PropertyValue::Vector(vec![1.0, 0.1, f32::NAN]),
        PropertyValue::Element(plain.clone()),
        PropertyValue::Element(element("", &[("title", s("t"))])),
        PropertyValue::Composite(Composite {
            uuid: "c1".into(),
            items: vec![plain.clone(), element("", &[("ref", reference.clone())])],
        }),
    ];
    // Every ambiguous key, in an element, in a composite item, in an object.
    for key in [
        "raisin:ref",
        "raisin:url",
        "created_at",
        "items",
        "uuid",
        "element_type",
        "content",
        "type",
        "coordinates",
    ] {
        let content = [(key, s("x")), ("other", date.clone())];
        values.push(PropertyValue::Element(element("u", &content)));
        values.push(PropertyValue::Composite(Composite {
            uuid: "c".into(),
            items: vec![element("", &content)],
        }));
        values.push(obj(&content));
        values.push(obj(&[
            ("inner", obj(&content)),
            ("list", PropertyValue::Array(vec![obj(&content)])),
        ]));
    }
    values.push(obj(&[
        (
            "refs",
            PropertyValue::Array(vec![reference.clone(), reference_with_path]),
        ),
        ("block", PropertyValue::Element(plain)),
        ("when", date),
        ("price", decimal),
        ("link", url),
        ("pin", point),
        ("$mixins", PropertyValue::Vector(vec![])),
        ("nan", PropertyValue::Float(f64::NAN)),
    ]));
    values.push(obj(&[
        ("uuid", s("u")),
        ("items", PropertyValue::Array(vec![])),
    ]));
    values.push(obj(&[("raisin:ref", PropertyValue::Integer(4))]));

    for value in values {
        let json = serde_json::to_value(&value).unwrap();
        let expected = PropertyValue::from_json(&json);
        let got = value.clone().into_json_round_trip();
        assert!(
            same(&got, &expected),
            "for {value:?}:\n got {got:?}\nwant {expected:?}"
        );
    }
}
