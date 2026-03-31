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

//! Tests for migration deserializers.

#[cfg(test)]
mod tests {
    use crate::migrations::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct TestOptional {
        #[serde(deserialize_with = "deserialize_optional_string_lenient")]
        field: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct TestRequired {
        #[serde(deserialize_with = "deserialize_string_lenient")]
        field: String,
    }

    #[test]
    fn test_optional_string_from_string() {
        let json = r#"{"field": "hello"}"#;
        let result: TestOptional = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, Some("hello".to_string()));
    }

    #[test]
    fn test_optional_string_from_false() {
        let json = r#"{"field": false}"#;
        let result: TestOptional = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, None);
    }

    #[test]
    fn test_optional_string_from_null() {
        let json = r#"{"field": null}"#;
        let result: TestOptional = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, None);
    }

    #[test]
    fn test_required_string_from_string() {
        let json = r#"{"field": "hello"}"#;
        let result: TestRequired = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, "hello");
    }

    #[test]
    fn test_required_string_from_false() {
        let json = r#"{"field": false}"#;
        let result: TestRequired = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, "");
    }

    #[test]
    fn test_required_string_from_null() {
        let json = r#"{"field": null}"#;
        let result: TestRequired = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, "");
    }

    #[derive(Debug, Deserialize)]
    struct TestVecString {
        #[serde(deserialize_with = "deserialize_vec_string_lenient")]
        field: Vec<String>,
    }

    #[test]
    fn test_vec_string_from_array() {
        let json = r#"{"field": ["a", "b", "c"]}"#;
        let result: TestVecString = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_vec_string_from_null() {
        let json = r#"{"field": null}"#;
        let result: TestVecString = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, Vec::<String>::new());
    }

    #[test]
    fn test_vec_string_from_empty_array() {
        let json = r#"{"field": []}"#;
        let result: TestVecString = serde_json::from_str(json).unwrap();
        assert_eq!(result.field, Vec::<String>::new());
    }

    // Tests for generic Vec<T> lenient deserializer
    #[derive(Debug, Deserialize, PartialEq)]
    struct TestItem {
        name: String,
    }

    #[derive(Debug, Deserialize)]
    struct TestVecGeneric {
        #[serde(deserialize_with = "deserialize_vec_lenient")]
        items: Vec<TestItem>,
    }

    #[test]
    fn test_vec_generic_from_array() {
        let json = r#"{"items": [{"name": "a"}, {"name": "b"}]}"#;
        let result: TestVecGeneric = serde_json::from_str(json).unwrap();
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].name, "a");
    }

    #[test]
    fn test_vec_generic_from_null() {
        let json = r#"{"items": null}"#;
        let result: TestVecGeneric = serde_json::from_str(json).unwrap();
        assert_eq!(result.items, Vec::<TestItem>::new());
    }

    #[test]
    fn test_vec_generic_from_empty_array() {
        let json = r#"{"items": []}"#;
        let result: TestVecGeneric = serde_json::from_str(json).unwrap();
        assert_eq!(result.items, Vec::<TestItem>::new());
    }
}
