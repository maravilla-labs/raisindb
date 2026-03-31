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

//! Tests for trigger matcher filters and glob matching

use super::filters::{glob_match, property_filter_matches};

#[test]
fn test_glob_match_exact() {
    assert!(glob_match("/content/users", "/content/users"));
    assert!(!glob_match("/content/users", "/content/posts"));
}

#[test]
fn test_glob_match_wildcard() {
    assert!(glob_match("/content/*", "/content/users"));
    assert!(glob_match("/content/*", "/content/posts"));
    assert!(!glob_match("/content/*", "/content/users/123"));
}

#[test]
fn test_glob_match_double_wildcard() {
    assert!(glob_match("/content/**", "/content/users"));
    assert!(glob_match("/content/**", "/content/users/123"));
    assert!(glob_match("/content/**", "/content/users/123/profile"));
}

#[test]
fn test_glob_match_double_wildcard_middle() {
    assert!(glob_match(
        "/content/**/profile",
        "/content/users/123/profile"
    ));
    assert!(!glob_match(
        "/content/**/profile",
        "/content/users/123/settings"
    ));
}

#[test]
fn test_property_filter_simple_match() {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    let mut props = HashMap::new();
    props.insert(
        "status".to_string(),
        PropertyValue::String("ready".to_string()),
    );
    props.insert("count".to_string(), PropertyValue::Integer(42));
    props.insert("enabled".to_string(), PropertyValue::Boolean(true));

    // String match
    assert!(property_filter_matches(
        &props,
        "status",
        &serde_json::json!("ready")
    ));
    assert!(!property_filter_matches(
        &props,
        "status",
        &serde_json::json!("draft")
    ));

    // Integer match
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!(42)
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!(99)
    ));

    // Boolean match
    assert!(property_filter_matches(
        &props,
        "enabled",
        &serde_json::json!(true)
    ));
    assert!(!property_filter_matches(
        &props,
        "enabled",
        &serde_json::json!(false)
    ));

    // Missing property
    assert!(!property_filter_matches(
        &props,
        "missing",
        &serde_json::json!("value")
    ));
}

#[test]
fn test_property_filter_nested_path() {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    // Create nested structure: file.metadata.storage_key
    let mut metadata = HashMap::new();
    metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String("abc123".to_string()),
    );
    metadata.insert("size".to_string(), PropertyValue::Integer(1024));

    let mut file = HashMap::new();
    file.insert("metadata".to_string(), PropertyValue::Object(metadata));
    file.insert(
        "name".to_string(),
        PropertyValue::String("test.jpg".to_string()),
    );

    let mut props = HashMap::new();
    props.insert("file".to_string(), PropertyValue::Object(file));
    props.insert(
        "title".to_string(),
        PropertyValue::String("My Image".to_string()),
    );

    // Top-level match
    assert!(property_filter_matches(
        &props,
        "title",
        &serde_json::json!("My Image")
    ));

    // Nested path match
    assert!(property_filter_matches(
        &props,
        "file.name",
        &serde_json::json!("test.jpg")
    ));
    assert!(property_filter_matches(
        &props,
        "file.metadata.storage_key",
        &serde_json::json!("abc123")
    ));
    assert!(property_filter_matches(
        &props,
        "file.metadata.size",
        &serde_json::json!(1024)
    ));

    // Non-existent nested path
    assert!(!property_filter_matches(
        &props,
        "file.metadata.missing",
        &serde_json::json!("value")
    ));
    assert!(!property_filter_matches(
        &props,
        "file.nonexistent.path",
        &serde_json::json!("value")
    ));
}

#[test]
fn test_property_filter_exists_operator() {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    let mut metadata = HashMap::new();
    metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String("abc123".to_string()),
    );

    let mut file = HashMap::new();
    file.insert("metadata".to_string(), PropertyValue::Object(metadata));

    let mut props = HashMap::new();
    props.insert("file".to_string(), PropertyValue::Object(file));
    props.insert(
        "status".to_string(),
        PropertyValue::String("ready".to_string()),
    );

    // $exists: true - property exists
    assert!(property_filter_matches(
        &props,
        "status",
        &serde_json::json!({"$exists": true})
    ));
    assert!(property_filter_matches(
        &props,
        "file.metadata.storage_key",
        &serde_json::json!({"$exists": true})
    ));

    // $exists: true - property doesn't exist
    assert!(!property_filter_matches(
        &props,
        "missing",
        &serde_json::json!({"$exists": true})
    ));
    assert!(!property_filter_matches(
        &props,
        "file.metadata.missing",
        &serde_json::json!({"$exists": true})
    ));

    // $exists: false - property doesn't exist
    assert!(property_filter_matches(
        &props,
        "missing",
        &serde_json::json!({"$exists": false})
    ));
    assert!(property_filter_matches(
        &props,
        "file.metadata.missing",
        &serde_json::json!({"$exists": false})
    ));

    // $exists: false - property exists
    assert!(!property_filter_matches(
        &props,
        "status",
        &serde_json::json!({"$exists": false})
    ));
}

#[test]
fn test_property_filter_comparison_operators() {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    let mut props = HashMap::new();
    props.insert("count".to_string(), PropertyValue::Integer(50));
    props.insert("score".to_string(), PropertyValue::Float(3.14));
    props.insert(
        "status".to_string(),
        PropertyValue::String("active".to_string()),
    );

    // $eq
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$eq": 50})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$eq": 100})
    ));

    // $ne
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$ne": 100})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$ne": 50})
    ));
    assert!(property_filter_matches(
        &props,
        "missing",
        &serde_json::json!({"$ne": 100})
    )); // missing != any value

    // $gt
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$gt": 40})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$gt": 50})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$gt": 60})
    ));

    // $gte
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$gte": 40})
    ));
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$gte": 50})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$gte": 60})
    ));

    // $lt
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$lt": 60})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$lt": 50})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$lt": 40})
    ));

    // $lte
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$lte": 60})
    ));
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$lte": 50})
    ));
    assert!(!property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$lte": 40})
    ));

    // $in
    assert!(property_filter_matches(
        &props,
        "status",
        &serde_json::json!({"$in": ["active", "pending"]})
    ));
    assert!(!property_filter_matches(
        &props,
        "status",
        &serde_json::json!({"$in": ["inactive", "deleted"]})
    ));
    assert!(property_filter_matches(
        &props,
        "count",
        &serde_json::json!({"$in": [10, 50, 100]})
    ));
}
