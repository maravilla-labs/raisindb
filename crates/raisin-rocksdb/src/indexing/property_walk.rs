//! The ONE recursive property-tree walker, and the ONE dot-path format.
//!
//! # Why this is shared rather than copied
//!
//! The reference index already walked nested properties
//! (`repositories/nodes/crud/indexing/reference_indexes.rs`), and the spatial
//! index did not — it matched a flat `PropertyValue::Geometry` at the top level
//! only, so a geometry inside an `Element`, `Object` or `Array` was stored,
//! reported the index healthy, and was invisible to every `ST_DWITHIN` /
//! `ST_DISTANCE` query.
//!
//! Fixing that by writing a second walker is how the two formats drift, and a
//! format mismatch between the writer and the tombstoner leaves index entries that
//! can never be shadowed — a stale hit that survives every update and every
//! delete. So there is exactly one traversal, here, and both index families select
//! their leaves out of it.
//!
//! # The path format, fixed
//!
//! Separator is `.` and only `.`:
//!
//! | where the value sits          | path            |
//! |-------------------------------|-----------------|
//! | top level                     | `location`      |
//! | inside an `Object`            | `venue.geo`     |
//! | inside an `Element`'s content | `hero.map_pin`  |
//! | inside an `Array`             | `stops.0.geo`   |
//!
//! Array indices are **zero-based** and appear as an ordinary segment.
//!
//! A top-level path is **byte-identical to the bare property name**, which is why
//! every index entry written before nested support existed stays valid and flat
//! data needs no migration.
//!
//! ## Known limitation, deliberately not fixed here
//!
//! A property name that itself contains a `.` is ambiguous against a nested path
//! and is NOT disambiguated — there is no escaping. This is the same limitation
//! the reference walker has always carried; inventing an escape now would change
//! the reference index's key format too.

use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// Walk a property tree and collect every leaf `select` accepts, paired with its
/// dot-format path.
///
/// Descends `Array`, `Object` and `Element` (whose fields live in `content`).
/// Results are sorted by path, so two calls over the same tree produce the same
/// order — `HashMap` iteration does not, and an unstable order would make a
/// "first N" cap (see `spatial_walk::walk_geometries_capped`) non-deterministic.
pub fn walk_properties<'a, T, F>(
    properties: &'a HashMap<String, PropertyValue>,
    select: F,
) -> Vec<(String, &'a T)>
where
    F: Fn(&'a PropertyValue) -> Option<&'a T> + Copy,
    T: ?Sized,
{
    fn visit<'a, T, F>(
        path: &str,
        value: &'a PropertyValue,
        select: F,
        out: &mut Vec<(String, &'a T)>,
    ) where
        F: Fn(&'a PropertyValue) -> Option<&'a T> + Copy,
        T: ?Sized,
    {
        // A selected leaf is terminal: nothing inside a matched value is walked
        // further. Both index families want the value, not its innards.
        if let Some(selected) = select(value) {
            out.push((path.to_string(), selected));
            return;
        }
        match value {
            PropertyValue::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    visit(&format!("{}.{}", path, i), item, select, out);
                }
            }
            PropertyValue::Object(obj) => {
                for (key, val) in obj {
                    visit(&format!("{}.{}", path, key), val, select, out);
                }
            }
            PropertyValue::Element(element) => {
                // Element blocks carry their fields in `content`; descend, or
                // element-nested values are invisible to every secondary index.
                for (key, val) in &element.content {
                    visit(&format!("{}.{}", path, key), val, select, out);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for (key, value) in properties {
        visit(key, value, select, &mut out);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::Element;

    fn strings(properties: &HashMap<String, PropertyValue>) -> Vec<(String, String)> {
        walk_properties(properties, |v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .into_iter()
        .map(|(p, s)| (p, s.to_string()))
        .collect()
    }

    #[test]
    fn top_level_path_is_the_bare_property_name() {
        let mut props = HashMap::new();
        props.insert("name".into(), PropertyValue::String("a".into()));
        assert_eq!(strings(&props), vec![("name".to_string(), "a".to_string())]);
    }

    #[test]
    fn object_array_and_element_paths() {
        let mut inner = HashMap::new();
        inner.insert("geo".into(), PropertyValue::String("o".into()));
        let mut element_content = HashMap::new();
        element_content.insert("map_pin".into(), PropertyValue::String("e".into()));

        let mut props = HashMap::new();
        props.insert("venue".into(), PropertyValue::Object(inner));
        props.insert(
            "hero".into(),
            PropertyValue::Element(Element {
                uuid: "u".into(),
                element_type: "t".into(),
                content: element_content,
            }),
        );
        props.insert(
            "stops".into(),
            PropertyValue::Array(vec![
                PropertyValue::String("s0".into()),
                PropertyValue::String("s1".into()),
            ]),
        );

        assert_eq!(
            strings(&props),
            vec![
                ("hero.map_pin".to_string(), "e".to_string()),
                ("stops.0".to_string(), "s0".to_string()),
                ("stops.1".to_string(), "s1".to_string()),
                ("venue.geo".to_string(), "o".to_string()),
            ]
        );
    }

    #[test]
    fn descends_three_levels() {
        let mut level3 = HashMap::new();
        level3.insert("geo".into(), PropertyValue::String("deep".into()));
        let mut level2 = HashMap::new();
        level2.insert(
            "spot".into(),
            PropertyValue::Array(vec![PropertyValue::Object(level3)]),
        );
        let mut props = HashMap::new();
        props.insert("venue".into(), PropertyValue::Object(level2));

        assert_eq!(
            strings(&props),
            vec![("venue.spot.0.geo".to_string(), "deep".to_string())]
        );
    }

    #[test]
    fn output_order_is_stable_across_calls() {
        let mut props = HashMap::new();
        for i in 0..32 {
            props.insert(format!("p{i:02}"), PropertyValue::String(i.to_string()));
        }
        assert_eq!(strings(&props), strings(&props));
    }
}
