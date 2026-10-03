// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Re-checking a PROPERTY_INDEX candidate against the node it names.
//!
//! The property index answers with CANDIDATES. Historically buggy writers (the
//! replica writer that omitted membership entries, merge apply, the old
//! in-place writer) left orphan LIVE entries behind, and the bounded reader
//! cannot tell an orphan from a real entry. For a user property that is
//! harmless: the planner keeps the JSON predicate as a residual filter, which
//! sees the decoded node and drops the phantom. A PSEUDO-property has no such
//! residual — `node_type = 'x'`, `name = …`, `created_at` ranges and
//! `IS_A`/`HAS_MIXIN` are answered by the index alone — so an orphan entry
//! there becomes a phantom row (and a phantom in `COUNT(*)`). See the Phase 6
//! amendment in `docs/perf/read-index-resolve-plan-critique.md`.
//!
//! So every scan that serves a pseudo-property from the index re-checks the
//! decoded node here, ONE function for all of them.

use raisin_models::nodes::{Node, INDEXED_MIXIN_KEY, INDEXED_SUPERTYPE_KEY};

/// Is `property_name` one of the index's pseudo-properties (anything the node
/// RECORD answers rather than its `properties` map)? All of them are spelled
/// with a leading `__`; treating every `__` name as pseudo errs towards the
/// re-check, which is always safe.
pub(crate) fn is_pseudo_property(property_name: &str) -> bool {
    property_name.starts_with("__")
}

/// Does `node`, as decoded, still carry `value` for the pseudo-property
/// `property_name`?
///
/// `value` is the index's text for the entry: the stored text for everything
/// except `__created_at` / `__updated_at`, which are decimal microseconds.
/// A user property, or a pseudo-property this function does not know, answers
/// `true` — never drop a row on a question it cannot decide.
pub(crate) fn node_still_matches(node: &Node, property_name: &str, value: &str) -> bool {
    match property_name {
        "__node_type" => node.node_type == value,
        "__name" => node.name == value,
        "__archetype" => node.archetype.as_deref() == Some(value),
        "__created_by" => node.created_by.as_deref() == Some(value),
        "__updated_by" => node.updated_by.as_deref() == Some(value),
        "__created_at" => micros_match(node.created_at, value),
        "__updated_at" => micros_match(node.updated_at, value),
        name if name == INDEXED_SUPERTYPE_KEY => {
            node.effective_supertypes().iter().any(|t| t == value)
        }
        name if name == INDEXED_MIXIN_KEY => node.effective_mixins().iter().any(|m| m == value),
        _ => true,
    }
}

/// `properties->>'key' = expected`, with the residual filter's text semantics:
/// a string member compares unquoted, a missing or null member never matches,
/// any other member compares as its JSON text.
///
/// For a PropertyIndexScan whose planner moved the driving JSON equality out of
/// the residual filter and into the scan (`verifies_value`). It must agree with
/// `eval::core::json_eval::json_member_text` exactly, or a lookup would return
/// rows the same predicate as a row filter rejects.
pub(crate) fn json_member_equals(node: &Node, key: &str, expected: &str) -> bool {
    use raisin_models::nodes::properties::PropertyValue;
    match node.properties.get(key) {
        None | Some(PropertyValue::Null) => false,
        Some(PropertyValue::String(s)) => s == expected,
        Some(other) => match serde_json::to_value(other) {
            Ok(serde_json::Value::Null) | Err(_) => false,
            Ok(serde_json::Value::String(s)) => s == expected,
            Ok(json) => json.to_string() == expected,
        },
    }
}

/// Is the node's timestamp the one the entry was written for? A node without
/// the timestamp cannot own an entry for it.
fn micros_match(at: Option<chrono::DateTime<chrono::Utc>>, value: &str) -> bool {
    match (at, value.parse::<i64>()) {
        (Some(at), Ok(micros)) => at.timestamp_micros() == micros,
        // An entry whose value is not decimal micros was not written by the
        // timestamp writer; it cannot be judged, so it is kept.
        (Some(_), Err(_)) => true,
        (None, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Node {
        Node {
            id: "n1".into(),
            name: "home".into(),
            node_type: "test:Page".into(),
            created_at: chrono::DateTime::from_timestamp(1_700_000_000, 1_000),
            ..Default::default()
        }
    }

    #[test]
    fn pseudo_properties_are_checked_against_the_record() {
        let n = node();
        assert!(node_still_matches(&n, "__node_type", "test:Page"));
        assert!(!node_still_matches(&n, "__node_type", "test:Other"));
        assert!(node_still_matches(&n, "__name", "home"));
        assert!(!node_still_matches(&n, "__name", "away"));
        assert!(node_still_matches(&n, "__created_at", "1700000000000001"));
        assert!(!node_still_matches(&n, "__created_at", "1700000000000000"));
        // Never written: an entry claiming one is an orphan.
        assert!(!node_still_matches(&n, "__updated_at", "1700000000000001"));
        assert!(!node_still_matches(&n, "__archetype", "x"));
    }

    #[test]
    fn json_member_equality_uses_text_semantics() {
        use raisin_models::nodes::properties::PropertyValue;
        let mut n = node();
        n.properties
            .insert("slug".into(), PropertyValue::String("x".into()));
        n.properties.insert("n".into(), PropertyValue::Integer(5));
        n.properties.insert("gone".into(), PropertyValue::Null);
        assert!(json_member_equals(&n, "slug", "x"));
        assert!(!json_member_equals(&n, "slug", "\"x\""));
        assert!(json_member_equals(&n, "n", "5"));
        assert!(!json_member_equals(&n, "gone", "null"));
        assert!(!json_member_equals(&n, "missing", "x"));
    }

    #[test]
    fn user_properties_are_left_to_the_residual_filter() {
        assert!(node_still_matches(&node(), "slug", "anything"));
        assert!(!is_pseudo_property("slug"));
        assert!(is_pseudo_property("__node_type"));
    }
}
