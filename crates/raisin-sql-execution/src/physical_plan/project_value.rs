//! The value a projected COLUMN reference yields (plan Phase 13b, 13d).
//!
//! A projection evaluates `t.col` to a [`Literal`] (`from_property_value`) and
//! converts the result back (`to_property_value`). For an object or array that
//! round trip goes through `serde_json`: the stored map is serialized to a
//! `Value` and re-classified by `PropertyValue::from_json` — for `SELECT *`
//! that is the whole `properties` map plus every object/array property column,
//! measured at ~10 % of a release-build point lookup.
//!
//! [`projected_column_value`] returns EXACTLY what the round trip returns:
//! every value that travels as JSON (objects, arrays, references, resources,
//! elements, composites) through `PropertyValue::into_json_round_trip` — the
//! one "what JSON gives back" rule, which keeps whatever provably comes back
//! unchanged and runs the real round trip only for the subtrees it cannot
//! prove — scalars and vectors as themselves, and the rest (dates, decimals,
//! URLs, geometries) through the literal conversion itself. The equivalence
//! test below pins the two to one answer.

use super::types::{from_property_value, to_property_value};
use raisin_models::nodes::properties::PropertyValue;

/// `to_property_value(&from_property_value(value)?)`, without the JSON round
/// trip where it is known what the round trip yields.
pub(crate) fn projected_column_value(value: &PropertyValue) -> Result<PropertyValue, String> {
    match value {
        PropertyValue::Null
        | PropertyValue::Boolean(_)
        | PropertyValue::Integer(_)
        | PropertyValue::Float(_)
        | PropertyValue::String(_)
        | PropertyValue::Vector(_)
        | PropertyValue::Array(_)
        | PropertyValue::Object(_)
        | PropertyValue::Reference(_)
        | PropertyValue::Resource(_)
        | PropertyValue::Composite(_)
        | PropertyValue::Element(_) => projected_column_value_owned(value.clone()),
        _ => to_property_value(&from_property_value(value)?),
    }
}

/// [`projected_column_value`] for a value the caller gives up: moved, not
/// cloned, when the round trip is known.
pub(crate) fn projected_column_value_owned(value: PropertyValue) -> Result<PropertyValue, String> {
    match value {
        // A scalar or a top-level vector maps to a `Literal` and back
        // unchanged (a top-level NaN stays a `Double`).
        PropertyValue::Null
        | PropertyValue::Boolean(_)
        | PropertyValue::Integer(_)
        | PropertyValue::Float(_)
        | PropertyValue::String(_)
        | PropertyValue::Vector(_) => Ok(value),
        // These become `Literal::JsonB(to_value(v))` and come back through
        // `from_json`: the JSON round trip, by definition.
        PropertyValue::Array(_)
        | PropertyValue::Object(_)
        | PropertyValue::Reference(_)
        | PropertyValue::Resource(_)
        | PropertyValue::Composite(_)
        | PropertyValue::Element(_) => Ok(value.into_json_round_trip()),
        // Dates, decimals, URLs and geometries are re-spelled by the
        // literal conversion itself.
        other => to_property_value(&from_property_value(&other)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Equal, or (NaN != NaN) rendered identically.
    fn same(a: &PropertyValue, b: &PropertyValue) -> bool {
        a == b || format!("{a:?}") == format!("{b:?}")
    }

    fn round_trip(value: &PropertyValue) -> PropertyValue {
        to_property_value(&from_property_value(value).unwrap()).unwrap()
    }

    fn obj(entries: &[(&str, PropertyValue)]) -> PropertyValue {
        PropertyValue::Object(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    /// The fast path answers exactly what the round trip answers, for values
    /// it takes AND values it hands to the round trip.
    #[test]
    fn equals_the_round_trip() {
        let s = |v: &str| PropertyValue::String(v.to_string());
        let date: PropertyValue =
            serde_json::from_value(serde_json::json!("2026-10-05T12:00:00Z")).unwrap();
        let values = vec![
            PropertyValue::Null,
            PropertyValue::Boolean(true),
            PropertyValue::Integer(-7),
            PropertyValue::Float(1.0),
            PropertyValue::Float(2.5),
            PropertyValue::Float(f64::NAN),
            s("2026-10-05T12:00:00Z"),
            s("10"),
            date.clone(),
            PropertyValue::Array(vec![
                s("a"),
                PropertyValue::Integer(1),
                PropertyValue::Float(1.0),
            ]),
            PropertyValue::Array(vec![PropertyValue::Float(f64::INFINITY)]),
            PropertyValue::Array(vec![date.clone()]),
            obj(&[
                ("title", s("x")),
                ("tags", PropertyValue::Array(vec![s("a"), s("b")])),
                ("nested", obj(&[("n", PropertyValue::Integer(3))])),
                ("ok", PropertyValue::Boolean(false)),
                ("none", PropertyValue::Null),
            ]),
            obj(&[("raisin:ref", s("id")), ("raisin:workspace", s("ws"))]),
            obj(&[("inner", obj(&[("raisin:ref", s("id"))]))]),
            obj(&[
                ("type", s("Point")),
                ("coordinates", PropertyValue::Array(vec![])),
            ]),
            obj(&[("element_type", s("hero")), ("title", s("t"))]),
            obj(&[("uuid", s("u")), ("items", PropertyValue::Array(vec![]))]),
            obj(&[("when", date)]),
            PropertyValue::Vector(vec![1.0, 2.5]),
            obj(&[("$mixins", PropertyValue::Vector(vec![]))]),
            obj(&[("v", PropertyValue::Vector(vec![1.0, 0.1, -3.0, f32::NAN]))]),
            PropertyValue::Array(vec![PropertyValue::Vector(vec![2.0])]),
        ];
        for value in values {
            let fast = projected_column_value(&value).unwrap();
            let moved = projected_column_value_owned(value.clone()).unwrap();
            let slow = round_trip(&value);
            assert!(same(&moved, &slow), "for {value:?}: {moved:?} vs {slow:?}");
            assert!(same(&fast, &slow), "for {value:?}: {fast:?} vs {slow:?}");
        }
    }

    #[test]
    fn a_plain_property_map_is_not_converted() {
        // A bare reference no longer sends the whole map through JSON: only a
        // subtree the round trip could re-classify does.
        let map = obj(&[
            ("title", PropertyValue::String("x".into())),
            ("rank", PropertyValue::Integer(1)),
            ("$mixins", PropertyValue::Vector(vec![])),
            (
                "refs",
                obj(&[("raisin:ref", PropertyValue::String("id".into()))]),
            ),
        ]);
        let fast = projected_column_value(&map).unwrap();
        assert!(same(&fast, &round_trip(&map)));
    }
}
