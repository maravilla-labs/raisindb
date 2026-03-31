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

//! Property filter utilities for trigger matching
//!
//! Provides glob matching, nested property access, and MongoDB-style
//! filter operators ($exists, $eq, $ne, $gt, $gte, $lt, $lte, $in).

use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// Glob pattern matching using the glob crate
///
/// Supports standard glob patterns:
/// - `*` matches any characters except `/`
/// - `**` matches any characters including `/` (recursive)
/// - `?` matches any single character
/// - `[...]` matches character classes
pub(super) fn glob_match(pattern: &str, text: &str) -> bool {
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..Default::default()
    };
    glob::Pattern::new(pattern)
        .map(|p| p.matches_with(text, options))
        .unwrap_or(false)
}

/// Get a nested property value using dot notation
///
/// Supports paths like "file.metadata.storage_key" to traverse
/// into nested PropertyValue::Object structures.
pub(super) fn get_nested_property<'a>(
    properties: &'a HashMap<String, PropertyValue>,
    path: &str,
) -> Option<&'a PropertyValue> {
    let mut current: Option<&PropertyValue> = None;
    let mut current_map = properties;

    for key in path.split('.') {
        current = current_map.get(key);
        match current {
            Some(PropertyValue::Object(obj)) => current_map = obj,
            Some(value) => return Some(value),
            None => return None,
        }
    }
    current
}

/// Check if a property filter matches
///
/// Supports:
/// - Simple value comparison: `"status": "ready"` -- exact match
/// - Nested paths: `"file.metadata.storage_key": "..."` -- dot notation for nested objects
/// - Operators:
///   - `$exists`: Check if property exists (e.g., `"file.metadata.storage_key": {"$exists": true}`)
///   - `$eq`: Exact equality (e.g., `"status": {"$eq": "ready"}`)
///   - `$ne`: Not equal (e.g., `"status": {"$ne": "draft"}`)
///   - `$gt`, `$gte`, `$lt`, `$lte`: Numeric comparisons
///   - `$in`: Value is in array (e.g., `"status": {"$in": ["ready", "published"]}`)
pub(super) fn property_filter_matches(
    properties: &HashMap<String, PropertyValue>,
    key: &str,
    expected: &serde_json::Value,
) -> bool {
    // Check if expected is an operator object
    if let Some(obj) = expected.as_object() {
        if let Some(result) = check_operator(properties, key, obj) {
            return result;
        }
        // No recognized operator, fall through to simple comparison
    }

    // Simple value comparison with nested path support
    let actual = get_nested_property(properties, key);
    actual
        .map(|v| compare_property_value(v, expected))
        .unwrap_or(false)
}

/// Check operator-based filters ($exists, $eq, $ne, $gt, $gte, $lt, $lte, $in)
///
/// Returns `Some(bool)` if a recognized operator was found, `None` otherwise.
fn check_operator(
    properties: &HashMap<String, PropertyValue>,
    key: &str,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<bool> {
    if let Some(exists_value) = obj.get("$exists") {
        let should_exist = exists_value.as_bool().unwrap_or(true);
        let does_exist = get_nested_property(properties, key).is_some();
        return Some(does_exist == should_exist);
    }

    if let Some(eq_value) = obj.get("$eq") {
        let actual = get_nested_property(properties, key);
        return Some(
            actual
                .map(|v| compare_property_value(v, eq_value))
                .unwrap_or(false),
        );
    }

    if let Some(ne_value) = obj.get("$ne") {
        let actual = get_nested_property(properties, key);
        // If property doesn't exist, it's "not equal" to any value
        return Some(
            actual
                .map(|v| !compare_property_value(v, ne_value))
                .unwrap_or(true),
        );
    }

    if let Some(gt_value) = obj.get("$gt") {
        let actual = get_nested_property(properties, key);
        return Some(
            actual
                .and_then(|v| compare_numeric(v, gt_value, |a, b| a > b))
                .unwrap_or(false),
        );
    }

    if let Some(gte_value) = obj.get("$gte") {
        let actual = get_nested_property(properties, key);
        return Some(
            actual
                .and_then(|v| compare_numeric(v, gte_value, |a, b| a >= b))
                .unwrap_or(false),
        );
    }

    if let Some(lt_value) = obj.get("$lt") {
        let actual = get_nested_property(properties, key);
        return Some(
            actual
                .and_then(|v| compare_numeric(v, lt_value, |a, b| a < b))
                .unwrap_or(false),
        );
    }

    if let Some(lte_value) = obj.get("$lte") {
        let actual = get_nested_property(properties, key);
        return Some(
            actual
                .and_then(|v| compare_numeric(v, lte_value, |a, b| a <= b))
                .unwrap_or(false),
        );
    }

    if let Some(in_value) = obj.get("$in") {
        if let Some(arr) = in_value.as_array() {
            let actual = get_nested_property(properties, key);
            return Some(
                actual
                    .map(|v| {
                        arr.iter()
                            .any(|candidate| compare_property_value(v, candidate))
                    })
                    .unwrap_or(false),
            );
        }
        return Some(false);
    }

    None
}

/// Compare a PropertyValue with a serde_json::Value
pub(super) fn compare_property_value(actual: &PropertyValue, expected: &serde_json::Value) -> bool {
    match actual {
        PropertyValue::String(s) => expected.as_str().map(|exp| s == exp).unwrap_or(false),
        PropertyValue::Integer(i) => expected.as_i64().map(|exp| *i == exp).unwrap_or(false),
        PropertyValue::Boolean(b) => expected.as_bool().map(|exp| *b == exp).unwrap_or(false),
        PropertyValue::Float(f) => expected
            .as_f64()
            .map(|exp| (*f - exp).abs() < f64::EPSILON)
            .unwrap_or(false),
        PropertyValue::Null => expected.is_null(),
        _ => false, // Other types not fully supported yet
    }
}

/// Compare numeric values using a comparison function
fn compare_numeric<F>(actual: &PropertyValue, expected: &serde_json::Value, cmp: F) -> Option<bool>
where
    F: Fn(f64, f64) -> bool,
{
    let actual_num = match actual {
        PropertyValue::Integer(i) => Some(*i as f64),
        PropertyValue::Float(f) => Some(*f),
        _ => None,
    }?;

    let expected_num = expected.as_f64()?;
    Some(cmp(actual_num, expected_num))
}
