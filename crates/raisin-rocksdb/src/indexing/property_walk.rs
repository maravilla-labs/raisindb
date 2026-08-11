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
//! The same rule covers WRITING: [`walk_properties_mut`] lives in this file, next
//! to the read walk, and the `walk_and_walk_mut_agree_on_paths` test is the guard
//! that the two stay byte-identical in what they address. A dot-path setter
//! written in another module is precisely the drift this header warns about.
//!
//! # The path format, fixed
//!
//! Separator is `.` and only `.`:
//!
//! | where the value sits            | path             |
//! |---------------------------------|------------------|
//! | top level                       | `location`       |
//! | inside an `Object`              | `venue.geo`      |
//! | inside an `Element`'s content   | `hero.map_pin`   |
//! | inside an `Array`               | `stops.0.geo`    |
//! | inside a `Composite`'s items    | `blocks.0.map_pin` |
//!
//! Array indices are **zero-based** and appear as an ordinary segment. A
//! `Composite` is a list of `Element` blocks, so it addresses exactly like an
//! array of elements: the item index, then the block's content field. The blocks
//! themselves are `Element` structs rather than `PropertyValue`s, so a block is
//! never offered to the selector — only what is inside it.
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

use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// Where the walk currently is, handed to the selector alongside the value.
///
/// # Why the selector needs more than the value
///
/// Geometry-ness and reference-ness are properties of the VALUE, so those two
/// selectors need nothing else. Schema-driven selection is not: whether a leaf is,
/// say, an encrypted field depends on the ELEMENT TYPE that declares it, which the
/// value cannot report. Passing the enclosing element type down the one traversal
/// is what lets such a selector exist without a second walker being written for it.
pub struct WalkCursor<'a> {
    /// Dot-format path of the value being offered, in the fixed format above.
    pub path: &'a str,
    /// element_type of the nearest enclosing Element/Composite item, if any.
    ///
    /// `None` at the top level and inside plain `Object`/`Array` containers that
    /// no element encloses. Nesting REPLACES it: inside `hero.body` where `body`
    /// is itself an Element, this is `body`'s type, not `hero`'s.
    pub enclosing_element_type: Option<&'a str>,
}

/// Walk a property tree and collect every leaf `select` accepts, paired with its
/// dot-format path.
///
/// Descends `Array`, `Object`, `Element` (whose fields live in `content`) and
/// `Composite` (whose `items` are Element blocks). Results are sorted by path, so
/// two calls over the same tree produce the same order — `HashMap` iteration does
/// not, and an unstable order would make a "first N" cap (see
/// `spatial_walk::walk_geometries_capped`) non-deterministic.
///
/// # Composite descent widens two live indexes
///
/// `Composite` was NOT descended before, while the fulltext planner
/// (`jobs/handlers/fulltext/plan.rs::collect_element_types`) always did. A
/// geometry or a reference inside a block collection was therefore stored,
/// reported healthy, and invisible to every `ST_DWITHIN` / `REFERENCES()` — the
/// same silent gap this walker was created to close for `Element`. Adding it here
/// means the spatial and reference indexes both start emitting keys for paths they
/// never emitted before, so nodes carrying composites need an index REBUILD to
/// become queryable; nothing already indexed changes or needs migrating.
pub fn walk_properties<'a, T, F>(
    properties: &'a HashMap<String, PropertyValue>,
    select: F,
) -> Vec<(String, &'a T)>
where
    F: Fn(&WalkCursor<'_>, &'a PropertyValue) -> Option<&'a T> + Copy,
    T: ?Sized,
{
    fn visit<'a, T, F>(
        path: &str,
        enclosing_element_type: Option<&str>,
        value: &'a PropertyValue,
        select: F,
        out: &mut Vec<(String, &'a T)>,
    ) where
        F: Fn(&WalkCursor<'_>, &'a PropertyValue) -> Option<&'a T> + Copy,
        T: ?Sized,
    {
        // A selected leaf is terminal: nothing inside a matched value is walked
        // further. Both index families want the value, not its innards.
        let cursor = WalkCursor {
            path,
            enclosing_element_type,
        };
        if let Some(selected) = select(&cursor, value) {
            out.push((path.to_string(), selected));
            return;
        }
        match value {
            PropertyValue::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    visit(
                        &format!("{}.{}", path, i),
                        enclosing_element_type,
                        item,
                        select,
                        out,
                    );
                }
            }
            PropertyValue::Object(obj) => {
                for (key, val) in obj {
                    visit(
                        &format!("{}.{}", path, key),
                        enclosing_element_type,
                        val,
                        select,
                        out,
                    );
                }
            }
            PropertyValue::Element(element) => {
                // Element blocks carry their fields in `content`; descend, or
                // element-nested values are invisible to every secondary index.
                for (key, val) in &element.content {
                    visit(
                        &format!("{}.{}", path, key),
                        Some(element.element_type.as_str()),
                        val,
                        select,
                        out,
                    );
                }
            }
            PropertyValue::Composite(composite) => {
                // Mirrors `fulltext/plan.rs::collect_element_types`: items, then
                // each block's content. The two must agree on what is reachable.
                for (i, item) in composite.items.iter().enumerate() {
                    for (key, val) in &item.content {
                        visit(
                            &format!("{}.{}.{}", path, i, key),
                            Some(item.element_type.as_str()),
                            val,
                            select,
                            out,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for (key, value) in properties {
        visit(key, None, value, select, &mut out);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The same traversal as [`walk_properties`], with mutable access to each value.
///
/// `visit` is offered every value the read walk offers its selector, at the
/// identical path and with the identical cursor. Return `true` to claim the value
/// as a leaf — the walk then does NOT descend into it, exactly as a selected leaf
/// is terminal in [`walk_properties`]. Return `false` to decline and let the walk
/// continue into its children. Returns how many values were claimed.
///
/// This exists because rewriting a leaf in place (vaulting a secret, say) needs a
/// dot-path SETTER, and there is none anywhere in the tree. Writing one in another
/// file is exactly the drift the module header warns about: a setter that
/// addresses `blocks.0.x` while the walker addresses `blocks.x` silently rewrites
/// the wrong leaf, or none. `walk_and_walk_mut_agree_on_paths` in this module's
/// tests IS the format-parity guard — it asserts both functions yield identical
/// path sets over a tree with every container kind, and it is the reason the two
/// live in one file.
///
/// Traversal order is deterministic (keys sorted at each level) but is NOT the
/// path-sorted order [`walk_properties`] returns; the guarantee is that the SET of
/// paths is identical, not the sequence.
pub fn walk_properties_mut<F>(
    properties: &mut HashMap<String, PropertyValue>,
    mut visit: F,
) -> usize
where
    F: FnMut(&WalkCursor<'_>, &mut PropertyValue) -> bool,
{
    fn descend_map<F>(
        path: &str,
        enclosing_element_type: Option<&str>,
        map: &mut HashMap<String, PropertyValue>,
        visit: &mut F,
        claimed: &mut usize,
    ) where
        F: FnMut(&WalkCursor<'_>, &mut PropertyValue) -> bool,
    {
        let mut keys: Vec<String> = map.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", path, key)
            };
            if let Some(value) = map.get_mut(&key) {
                visit_mut(&child_path, enclosing_element_type, value, visit, claimed);
            }
        }
    }

    fn descend_blocks<F>(path: &str, blocks: &mut [Element], visit: &mut F, claimed: &mut usize)
    where
        F: FnMut(&WalkCursor<'_>, &mut PropertyValue) -> bool,
    {
        for (i, item) in blocks.iter_mut().enumerate() {
            let Element {
                element_type,
                content,
                ..
            } = item;
            descend_map(
                &format!("{}.{}", path, i),
                Some(element_type.as_str()),
                content,
                visit,
                claimed,
            );
        }
    }

    fn visit_mut<F>(
        path: &str,
        enclosing_element_type: Option<&str>,
        value: &mut PropertyValue,
        visit: &mut F,
        claimed: &mut usize,
    ) where
        F: FnMut(&WalkCursor<'_>, &mut PropertyValue) -> bool,
    {
        {
            let cursor = WalkCursor {
                path,
                enclosing_element_type,
            };
            if visit(&cursor, value) {
                *claimed += 1;
                return;
            }
        }
        match value {
            PropertyValue::Array(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    visit_mut(
                        &format!("{}.{}", path, i),
                        enclosing_element_type,
                        item,
                        visit,
                        claimed,
                    );
                }
            }
            PropertyValue::Object(obj) => {
                descend_map(path, enclosing_element_type, obj, visit, claimed);
            }
            PropertyValue::Element(element) => {
                let Element {
                    element_type,
                    content,
                    ..
                } = element;
                descend_map(path, Some(element_type.as_str()), content, visit, claimed);
            }
            PropertyValue::Composite(composite) => {
                descend_blocks(path, &mut composite.items, visit, claimed);
            }
            _ => {}
        }
    }

    let mut claimed = 0;
    descend_map("", None, properties, &mut visit, &mut claimed);
    claimed
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::{Composite, Element};

    fn strings(properties: &HashMap<String, PropertyValue>) -> Vec<(String, String)> {
        walk_properties(properties, |_, v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .into_iter()
        .map(|(p, s)| (p, s.to_string()))
        .collect()
    }

    fn element(element_type: &str, content: HashMap<String, PropertyValue>) -> Element {
        Element {
            uuid: String::new(),
            element_type: element_type.into(),
            content,
        }
    }

    /// `(path, enclosing element type)` for every String leaf, as the READ walk
    /// sees it. Recorded through a side channel rather than returned, because a
    /// selector's return borrows the VALUE (`'a`) while the cursor only lives for
    /// the call — which is exactly the intended contract.
    fn enclosing_types_of_string_leaves(
        properties: &HashMap<String, PropertyValue>,
    ) -> Vec<(String, String)> {
        let seen = std::cell::RefCell::new(Vec::new());
        walk_properties(properties, |cursor, v| {
            if let PropertyValue::String(s) = v {
                seen.borrow_mut().push((
                    cursor.path.to_string(),
                    cursor.enclosing_element_type.unwrap_or("-").to_string(),
                ));
                Some(s.as_str())
            } else {
                None
            }
        });
        seen.into_inner()
    }

    fn one(key: &str, value: &str) -> HashMap<String, PropertyValue> {
        let mut map = HashMap::new();
        map.insert(key.to_string(), PropertyValue::String(value.into()));
        map
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

    /// A `Composite` addresses like an array of elements: index, then content key.
    #[test]
    fn composite_items_are_indexed_by_position_then_content_key() {
        let mut props = HashMap::new();
        props.insert(
            "blocks".into(),
            PropertyValue::Composite(Composite {
                uuid: "c".into(),
                items: vec![
                    element("demo:Hero", one("map_pin", "a")),
                    element("demo:Stop", one("geo", "b")),
                ],
            }),
        );

        assert_eq!(
            strings(&props),
            vec![
                ("blocks.0.map_pin".to_string(), "a".to_string()),
                ("blocks.1.geo".to_string(), "b".to_string()),
            ]
        );
    }

    #[test]
    fn composite_nested_inside_an_element_keeps_descending() {
        let mut outer_content = HashMap::new();
        outer_content.insert(
            "body".into(),
            PropertyValue::Composite(Composite {
                uuid: "c".into(),
                items: vec![element("demo:Stop", one("geo", "deep"))],
            }),
        );
        let mut props = HashMap::new();
        props.insert(
            "hero".into(),
            PropertyValue::Element(element("demo:Hero", outer_content)),
        );

        assert_eq!(
            strings(&props),
            vec![("hero.body.0.geo".to_string(), "deep".to_string())]
        );
    }

    #[test]
    fn cursor_reports_the_nearest_enclosing_element_type() {
        let mut hero_content = HashMap::new();
        hero_content.insert("title".into(), PropertyValue::String("t".into()));
        hero_content.insert(
            "body".into(),
            PropertyValue::Composite(Composite {
                uuid: "c".into(),
                items: vec![element("demo:Stop", one("geo", "g"))],
            }),
        );

        let mut props = HashMap::new();
        props.insert("plain".into(), PropertyValue::String("p".into()));
        props.insert(
            "hero".into(),
            PropertyValue::Element(element("demo:Hero", hero_content)),
        );

        let mut seen = enclosing_types_of_string_leaves(&props);
        seen.sort();

        assert_eq!(
            seen,
            vec![
                ("hero.body.0.geo".to_string(), "demo:Stop".to_string()),
                ("hero.title".to_string(), "demo:Hero".to_string()),
                ("plain".to_string(), "-".to_string()),
            ]
        );
    }

    /// A rich tree exercising every container kind, used by the parity guard.
    fn rich_fixture() -> HashMap<String, PropertyValue> {
        let mut deep = HashMap::new();
        deep.insert("geo".into(), PropertyValue::String("deep".into()));

        let mut hero_content = HashMap::new();
        hero_content.insert("title".into(), PropertyValue::String("h".into()));
        hero_content.insert("nested".into(), PropertyValue::Object(deep));
        hero_content.insert(
            "body".into(),
            PropertyValue::Composite(Composite {
                uuid: "c1".into(),
                items: vec![
                    element("demo:Stop", one("geo", "b0")),
                    element("demo:Text", one("text", "b1")),
                ],
            }),
        );

        let mut props = HashMap::new();
        props.insert("location".into(), PropertyValue::String("top".into()));
        props.insert("count".into(), PropertyValue::Integer(3));
        props.insert(
            "venue".into(),
            PropertyValue::Object(one("geo", "venue-geo")),
        );
        props.insert(
            "hero".into(),
            PropertyValue::Element(element("demo:Hero", hero_content)),
        );
        props.insert(
            "stops".into(),
            PropertyValue::Array(vec![
                PropertyValue::String("s0".into()),
                PropertyValue::Element(element("demo:Stop", one("geo", "s1"))),
                PropertyValue::Composite(Composite {
                    uuid: "c2".into(),
                    items: vec![element("demo:Stop", one("geo", "s2"))],
                }),
            ]),
        );
        props
    }

    /// THE FORMAT-PARITY GUARD.
    ///
    /// `walk_properties_mut` must address exactly what `walk_properties` reads. If
    /// this ever fails, a rewrite is landing on a path no index entry was ever
    /// written for (or vice versa) — the silent-wrong-data shape the module header
    /// exists to prevent.
    #[test]
    fn walk_and_walk_mut_agree_on_paths() {
        let mut props = rich_fixture();

        let read: Vec<String> = walk_properties(&props, |_, v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .into_iter()
        .map(|(p, _)| p)
        .collect();

        let mut written = Vec::new();
        let claimed = walk_properties_mut(&mut props, |cursor, v| {
            if matches!(v, PropertyValue::String(_)) {
                written.push(cursor.path.to_string());
                true
            } else {
                false
            }
        });
        written.sort();

        assert_eq!(read, written);
        assert_eq!(claimed, read.len());
        assert!(!read.is_empty());
    }

    /// Parity must also hold when the selector claims a CONTAINER, i.e. when both
    /// walks have to stop descending at the same place.
    #[test]
    fn walk_and_walk_mut_agree_when_a_container_is_terminal() {
        let mut props = rich_fixture();

        let read: Vec<String> = walk_properties(&props, |_, v| match v {
            PropertyValue::Element(e) => Some(e.element_type.as_str()),
            _ => None,
        })
        .into_iter()
        .map(|(p, _)| p)
        .collect();

        let mut written = Vec::new();
        walk_properties_mut(&mut props, |cursor, v| {
            if matches!(v, PropertyValue::Element(_)) {
                written.push(cursor.path.to_string());
                true
            } else {
                false
            }
        });
        written.sort();

        assert_eq!(read, written);
        assert_eq!(read, vec!["hero".to_string(), "stops.1".to_string()]);
    }

    /// The cursor a rewrite sees must carry the same enclosing type the read walk
    /// reports, or schema-driven selection would rewrite a different set.
    #[test]
    fn walk_mut_cursor_carries_the_enclosing_element_type() {
        let mut props = rich_fixture();

        let mut read = enclosing_types_of_string_leaves(&props);
        read.sort();

        let mut written = Vec::new();
        walk_properties_mut(&mut props, |cursor, v| {
            if matches!(v, PropertyValue::String(_)) {
                written.push((
                    cursor.path.to_string(),
                    cursor.enclosing_element_type.unwrap_or("-").to_string(),
                ));
                true
            } else {
                false
            }
        });
        written.sort();

        assert_eq!(read, written);
    }

    /// The point of the mut walk: rewrite a leaf in place, addressed by path.
    #[test]
    fn walk_mut_rewrites_leaves_in_place() {
        let mut props = rich_fixture();

        walk_properties_mut(&mut props, |cursor, value| {
            if cursor.path == "hero.body.0.geo" {
                *value = PropertyValue::String("vaulted".into());
                true
            } else {
                false
            }
        });

        let found = walk_properties(&props, |_, v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .into_iter()
        .find(|(p, _)| p == "hero.body.0.geo")
        .map(|(_, s)| s.to_string());

        assert_eq!(found, Some("vaulted".to_string()));
    }
}
