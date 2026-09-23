// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Canonical JSON: the one serialization every digest is computed over.
//!
//! Object keys are sorted EXPLICITLY and recursively. That is not decoration:
//! the core workspace enables `serde_json/preserve_order`, so the map order of a
//! `serde_json::Value` there is insertion order, while a guest built without
//! that feature iterates a `BTreeMap`. A digest over `to_string` would differ
//! between the two builds for the same value.

use serde_json::Value;

/// Serialize `value` compactly with every object's keys sorted by byte order.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut out);
    out
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(&map[key.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::String(s) => write_string(s, out),
        other => out.push_str(&other.to_string()),
    }
}

fn write_string(s: &str, out: &mut String) {
    // serde_json's string escaping is canonical (no optional escapes).
    out.push_str(&Value::String(s.to_owned()).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_output() {
        let a = json!({"b": 1, "a": {"y": [1, {"d": 2, "c": 3}], "x": null}});
        let b: Value =
            serde_json::from_str(r#"{"a":{"x":null,"y":[1,{"c":3,"d":2}]},"b":1}"#).unwrap();
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(
            canonical_json(&a),
            r#"{"a":{"x":null,"y":[1,{"c":3,"d":2}]},"b":1}"#
        );
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(canonical_json(&json!({"k": "a\"b\n"})), r#"{"k":"a\"b\n"}"#);
    }
}
