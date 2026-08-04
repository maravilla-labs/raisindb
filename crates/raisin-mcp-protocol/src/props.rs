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

//! Small readers for a node's serialized property map.
//!
//! Shared by the server-side [`crate::server::McpServerDescriptor`] and the
//! client-side [`crate::client::McpConnectionDescriptor`]. Both parse a
//! `raisin:*` node out of the same `serde_json::to_value(&node.properties)`
//! shape, and a second private copy of "read a string property" is how the two
//! sides end up disagreeing about whether an empty string counts as absent.

use serde_json::Value;

/// Read a string property.
pub fn str_prop(props: &Value, key: &str) -> Option<String> {
    props.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Read a string property, treating `""` as absent.
///
/// A cleared form field arrives as an empty string, not a missing key, so every
/// required-field check wants this rather than [`str_prop`].
pub fn non_empty_str_prop(props: &Value, key: &str) -> Option<String> {
    str_prop(props, key).filter(|s| !s.is_empty())
}

/// Read a boolean property.
pub fn bool_prop(props: &Value, key: &str) -> Option<bool> {
    props.get(key).and_then(Value::as_bool)
}

/// Read an unsigned-integer property.
pub fn u64_prop(props: &Value, key: &str) -> Option<u64> {
    let value = props.get(key)?;
    // JSON numbers off a node property may arrive as f64 even when integral.
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
}

/// Read a string-array property (empty when absent or non-array).
pub fn str_array_prop(props: &Value, key: &str) -> Vec<String> {
    props
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Read an object property, or `Value::Null` when absent.
pub fn obj_prop(props: &Value, key: &str) -> Value {
    props
        .get(key)
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_string_counts_as_absent_only_for_the_non_empty_reader() {
        let props = json!({ "slug": "" });
        assert_eq!(str_prop(&props, "slug"), Some(String::new()));
        assert_eq!(non_empty_str_prop(&props, "slug"), None);
    }

    #[test]
    fn integral_floats_read_as_integers() {
        // A number written through the JSON property path can come back as f64.
        let props = json!({ "a": 30.0, "b": 30, "c": -1 });
        assert_eq!(u64_prop(&props, "a"), Some(30));
        assert_eq!(u64_prop(&props, "b"), Some(30));
        assert_eq!(u64_prop(&props, "c"), None);
    }

    #[test]
    fn non_arrays_and_non_objects_degrade_to_empty() {
        let props = json!({ "scopes": "not-an-array", "cfg": 5 });
        assert!(str_array_prop(&props, "scopes").is_empty());
        assert_eq!(obj_prop(&props, "cfg"), Value::Null);
    }
}
