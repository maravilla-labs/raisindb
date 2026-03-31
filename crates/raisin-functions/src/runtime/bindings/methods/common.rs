// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Common helper functions for method bindings

use serde_json::Value;

/// Helper to parse a JSON string or return the value as-is.
///
/// If the input is a string containing valid JSON, it will be parsed.
/// Otherwise, the original value is returned unchanged.
///
/// This is useful when receiving values from JavaScript/Python bindings
/// that may pass JSON as a string.
pub fn parse_json_or_value(val: Value) -> Value {
    match val {
        Value::String(s) => {
            serde_json::from_str(&s).unwrap_or(Value::Object(serde_json::Map::new()))
        }
        other => other,
    }
}
