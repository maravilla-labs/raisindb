// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversion functions between Starlark and serde_json values

use starlark::values::{Heap, Value};

/// Convert a Starlark Value to serde_json::Value
pub(super) fn starlark_value_to_json(value: Value) -> serde_json::Value {
    if value.is_none() {
        return serde_json::Value::Null;
    }

    if let Some(b) = value.unpack_bool() {
        return serde_json::Value::Bool(b);
    }

    if let Some(n) = value.unpack_i32() {
        return serde_json::json!(n);
    }

    if let Some(s) = value.unpack_str() {
        // Check if it's JSON
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
            return parsed;
        }
        return serde_json::Value::String(s.to_string());
    }

    // For lists, convert each element
    if let Some(list) = starlark::values::list::ListRef::from_value(value) {
        let arr: Vec<serde_json::Value> = list.iter().map(starlark_value_to_json).collect();
        return serde_json::Value::Array(arr);
    }

    // For dicts, convert to object
    if let Some(dict) = starlark::values::dict::DictRef::from_value(value) {
        let mut obj = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let key = k
                .unpack_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| k.to_string());
            obj.insert(key, starlark_value_to_json(v));
        }
        return serde_json::Value::Object(obj);
    }

    // For structs, try to convert to object
    let string_repr = value.to_string();
    serde_json::from_str(&string_repr).unwrap_or(serde_json::Value::String(string_repr))
}

/// Convert serde_json::Value to Starlark Value
pub(super) fn json_to_starlark<'v>(heap: &'v Heap, val: &serde_json::Value) -> Value<'v> {
    match val {
        serde_json::Value::Null => Value::new_none(),
        serde_json::Value::Bool(b) => heap.alloc(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                heap.alloc(i as i32)
            } else if let Some(f) = n.as_f64() {
                heap.alloc(f)
            } else {
                Value::new_none()
            }
        }
        serde_json::Value::String(s) => heap.alloc(s.as_str()),
        serde_json::Value::Array(arr) => {
            let items: Vec<Value> = arr.iter().map(|v| json_to_starlark(heap, v)).collect();
            heap.alloc(items)
        }
        serde_json::Value::Object(obj) => {
            // Build a dict using SmallMap
            let mut dict =
                starlark::values::dict::Dict::new(starlark::collections::SmallMap::new());
            for (k, v) in obj {
                let key = heap.alloc(k.as_str());
                let val = json_to_starlark(heap, v);
                dict.insert_hashed(key.get_hashed().unwrap(), val);
            }
            heap.alloc(dict)
        }
    }
}
