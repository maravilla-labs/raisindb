//! Tests for JSON_QUERY function
//!
//! Covers all three phases:
//! - Phase 1: basic 2-argument extraction
//! - Phase 2: wrapper clause variants
//! - Phase 3: ON EMPTY / ON ERROR handling

use super::*;
use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::json;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn eval_json_query(json: serde_json::Value, path: &str) -> Result<Literal, Error> {
    let func = JsonQueryFunction;
    let json_expr = TypedExpr::new(Expr::Literal(Literal::JsonB(json)), DataType::JsonB);
    let path_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(path.to_string())),
        DataType::Text,
    );
    let row = Row::new();
    func.evaluate(&[json_expr, path_expr], &row)
}

fn eval_json_query_with_wrapper(
    json: serde_json::Value,
    path: &str,
    wrapper: &str,
) -> Result<Literal, Error> {
    let func = JsonQueryFunction;
    let json_expr = TypedExpr::new(Expr::Literal(Literal::JsonB(json)), DataType::JsonB);
    let path_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(path.to_string())),
        DataType::Text,
    );
    let wrapper_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(wrapper.to_string())),
        DataType::Text,
    );
    let row = Row::new();
    func.evaluate(&[json_expr, path_expr, wrapper_expr], &row)
}

fn eval_json_query_with_on_empty(
    json: serde_json::Value,
    path: &str,
    wrapper: &str,
    on_empty: &str,
) -> Result<Literal, Error> {
    let func = JsonQueryFunction;
    let json_expr = TypedExpr::new(Expr::Literal(Literal::JsonB(json)), DataType::JsonB);
    let path_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(path.to_string())),
        DataType::Text,
    );
    let wrapper_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(wrapper.to_string())),
        DataType::Text,
    );
    let on_empty_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(on_empty.to_string())),
        DataType::Text,
    );
    let row = Row::new();
    func.evaluate(&[json_expr, path_expr, wrapper_expr, on_empty_expr], &row)
}

fn eval_json_query_full(
    json: serde_json::Value,
    path: &str,
    wrapper: &str,
    on_empty: &str,
    on_error: &str,
) -> Result<Literal, Error> {
    let func = JsonQueryFunction;
    let json_expr = TypedExpr::new(Expr::Literal(Literal::JsonB(json)), DataType::JsonB);
    let path_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(path.to_string())),
        DataType::Text,
    );
    let wrapper_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(wrapper.to_string())),
        DataType::Text,
    );
    let on_empty_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(on_empty.to_string())),
        DataType::Text,
    );
    let on_error_expr = TypedExpr::new(
        Expr::Literal(Literal::Text(on_error.to_string())),
        DataType::Text,
    );
    let row = Row::new();
    func.evaluate(
        &[
            json_expr,
            path_expr,
            wrapper_expr,
            on_empty_expr,
            on_error_expr,
        ],
        &row,
    )
}

// ---------------------------------------------------------------------------
// Phase 1 - basic extraction
// ---------------------------------------------------------------------------

#[test]
fn test_json_query_extract_object() {
    let json = json!({
        "user": {
            "name": "Alice",
            "age": 30
        }
    });
    let result = eval_json_query(json, "$.user").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({"name": "Alice", "age": 30}));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_extract_array() {
    let json = json!({
        "tags": ["rust", "sql", "json"]
    });
    let result = eval_json_query(json, "$.tags").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!(["rust", "sql", "json"]));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_scalar_returns_null() {
    // JSON_QUERY returns NULL for scalar values (use JSON_VALUE instead)
    let json = json!({"name": "Alice"});
    let result = eval_json_query(json, "$.name").unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_null_value() {
    let json = json!({"data": null});
    let result = eval_json_query(json, "$.data").unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_missing_path() {
    let json = json!({"name": "Alice"});
    let result = eval_json_query(json, "$.missing").unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_nested_object() {
    let json = json!({
        "user": {
            "profile": {
                "email": "alice@example.com",
                "verified": true
            }
        }
    });
    let result = eval_json_query(json, "$.user.profile").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(
                value,
                json!({
                    "email": "alice@example.com",
                    "verified": true
                })
            );
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_null_input() {
    let func = JsonQueryFunction;
    let null_expr = TypedExpr::new(
        Expr::Literal(Literal::Null),
        DataType::Nullable(Box::new(DataType::JsonB)),
    );
    let path_expr = TypedExpr::new(
        Expr::Literal(Literal::Text("$.path".to_string())),
        DataType::Text,
    );
    let row = Row::new();
    let result = func.evaluate(&[null_expr, path_expr], &row).unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_invalid_jsonpath() {
    // With default NULL ON ERROR, invalid JSONPath returns NULL
    let json = json!({"name": "Alice"});
    let result = eval_json_query(json, "$.[invalid").unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_array_index() {
    let json = json!({
        "items": [
            {"id": 1, "name": "Item 1"},
            {"id": 2, "name": "Item 2"}
        ]
    });
    let result = eval_json_query(json, "$.items[0]").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({"id": 1, "name": "Item 1"}));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

// ---------------------------------------------------------------------------
// Phase 2 - wrapper clause tests
// ---------------------------------------------------------------------------

#[test]
fn test_json_query_with_wrapper_single_match() {
    // WITH WRAPPER: Always wraps, even single match
    let json = json!({"item": {"id": 1}});
    let result = eval_json_query_with_wrapper(json, "$.item", "WITH WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            // Should be wrapped in array
            assert_eq!(value, json!([{"id": 1}]));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_with_wrapper_multiple_matches() {
    // WITH WRAPPER: Wraps all matches in array
    let json = json!([{"id": 1}, {"id": 2}, {"id": 3}]);
    let result = eval_json_query_with_wrapper(json, "$[*]", "WITH WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([{"id": 1}, {"id": 2}, {"id": 3}]));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_without_wrapper_single_match() {
    // WITHOUT WRAPPER: Returns single match as-is
    let json = json!([{"id": 1}]);
    let result = eval_json_query_with_wrapper(json, "$[0]", "WITHOUT WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({"id": 1}));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_without_wrapper_multiple_matches() {
    // WITHOUT WRAPPER: Returns NULL for multiple matches
    let json = json!([{"id": 1}, {"id": 2}]);
    let result = eval_json_query_with_wrapper(json, "$[*]", "WITHOUT WRAPPER").unwrap();

    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_conditional_wrapper_single_match() {
    // CONDITIONAL WRAPPER: Single match not wrapped
    let json = json!([{"id": 1}]);
    let result = eval_json_query_with_wrapper(json, "$[0]", "WITH CONDITIONAL WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({"id": 1}));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_conditional_wrapper_multiple_matches() {
    // CONDITIONAL WRAPPER: Multiple matches wrapped in array
    let json = json!([{"id": 1}, {"id": 2}, {"id": 3}]);
    let result = eval_json_query_with_wrapper(json, "$[*]", "WITH CONDITIONAL WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([{"id": 1}, {"id": 2}, {"id": 3}]));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_wrapper_clause_case_insensitive() {
    // Test various case and format variations
    let json = json!([{"id": 1}, {"id": 2}]);

    // lowercase
    let result = eval_json_query_with_wrapper(json.clone(), "$[*]", "with wrapper").unwrap();
    assert!(matches!(result, Literal::JsonB(_)));

    // underscore format
    let result = eval_json_query_with_wrapper(json.clone(), "$[*]", "WITH_WRAPPER").unwrap();
    assert!(matches!(result, Literal::JsonB(_)));

    // mixed case
    let result = eval_json_query_with_wrapper(json, "$[*]", "With Wrapper").unwrap();
    assert!(matches!(result, Literal::JsonB(_)));
}

#[test]
fn test_json_query_wrapper_clause_invalid() {
    let json = json!([{"id": 1}]);
    let result = eval_json_query_with_wrapper(json, "$[*]", "INVALID CLAUSE");

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid wrapper clause"));
}

#[test]
fn test_json_query_wrapper_with_scalars_filtered() {
    // Wrapper clauses should filter out scalar values (JSON_QUERY only returns objects/arrays)
    let json = json!({"items": [{"id": 1}, "scalar_string", 123, {"id": 2}]});
    let result = eval_json_query_with_wrapper(json, "$.items[*]", "WITH WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            // Only objects should be included
            assert_eq!(value, json!([{"id": 1}, {"id": 2}]));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

#[test]
fn test_json_query_default_is_without_wrapper() {
    // 2-arg form should behave like WITHOUT WRAPPER
    let json = json!([{"id": 1}, {"id": 2}]);

    // Call with 2 args (no wrapper clause)
    let result_2arg = eval_json_query(json.clone(), "$[*]").unwrap();

    // Call with 3 args explicitly specifying WITHOUT WRAPPER
    let result_3arg = eval_json_query_with_wrapper(json, "$[*]", "WITHOUT WRAPPER").unwrap();

    // Both should return NULL for multiple matches
    assert!(matches!(result_2arg, Literal::Null));
    assert!(matches!(result_3arg, Literal::Null));
}

#[test]
fn test_json_query_wrapper_with_nested_paths() {
    // Test wrapper clauses with nested JSONPath queries
    let json = json!({
        "users": [
            {"name": "Alice", "profile": {"age": 30}},
            {"name": "Bob", "profile": {"age": 25}}
        ]
    });

    // Extract all profiles with wrapper
    let result = eval_json_query_with_wrapper(json, "$.users[*].profile", "WITH WRAPPER").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([{"age": 30}, {"age": 25}]));
        }
        _ => panic!("Expected JsonB literal"),
    }
}

// ---------------------------------------------------------------------------
// Phase 3 - ON EMPTY / ON ERROR tests
// ---------------------------------------------------------------------------

#[test]
fn test_json_query_on_empty_null() {
    // NULL ON EMPTY (default): Returns NULL for missing path
    let json = json!({"name": "test"});
    let result =
        eval_json_query_with_on_empty(json, "$.missing", "WITHOUT WRAPPER", "NULL").unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_on_empty_error() {
    // ERROR ON EMPTY: Raises error for missing path
    let json = json!({"name": "test"});
    let result = eval_json_query_with_on_empty(json, "$.missing", "WITHOUT WRAPPER", "ERROR");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("does not exist or result is empty"));
}

#[test]
fn test_json_query_on_empty_empty_array() {
    // EMPTY ARRAY ON EMPTY: Returns [] for missing path
    let json = json!({"name": "test"});
    let result =
        eval_json_query_with_on_empty(json, "$.missing", "WITHOUT WRAPPER", "EMPTY ARRAY").unwrap();
    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([]));
        }
        _ => panic!("Expected JsonB with empty array"),
    }
}

#[test]
fn test_json_query_on_empty_empty_object() {
    // EMPTY OBJECT ON EMPTY: Returns {} for missing path
    let json = json!({"name": "test"});
    let result =
        eval_json_query_with_on_empty(json, "$.missing", "WITHOUT WRAPPER", "EMPTY OBJECT")
            .unwrap();
    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({}));
        }
        _ => panic!("Expected JsonB with empty object"),
    }
}

#[test]
fn test_json_query_on_error_null() {
    // NULL ON ERROR (default): Returns NULL for invalid JSONPath
    let json = json!({"name": "test"});
    let result =
        eval_json_query_full(json, "$.[invalid", "WITHOUT WRAPPER", "NULL", "NULL").unwrap();
    assert!(matches!(result, Literal::Null));
}

#[test]
fn test_json_query_on_error_error() {
    // ERROR ON ERROR: Propagates error for invalid JSONPath
    let json = json!({"name": "test"});
    let result = eval_json_query_full(json, "$.[invalid", "WITHOUT WRAPPER", "NULL", "ERROR");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid JSONPath"));
}

#[test]
fn test_json_query_on_error_empty_array() {
    // EMPTY ARRAY ON ERROR: Returns [] for invalid JSONPath
    let json = json!({"name": "test"});
    let result =
        eval_json_query_full(json, "$.[invalid", "WITHOUT WRAPPER", "NULL", "EMPTY ARRAY").unwrap();
    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([]));
        }
        _ => panic!("Expected JsonB with empty array"),
    }
}

#[test]
fn test_json_query_on_error_empty_object() {
    // EMPTY OBJECT ON ERROR: Returns {} for invalid JSONPath
    let json = json!({"name": "test"});
    let result = eval_json_query_full(
        json,
        "$.[invalid",
        "WITHOUT WRAPPER",
        "NULL",
        "EMPTY OBJECT",
    )
    .unwrap();
    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({}));
        }
        _ => panic!("Expected JsonB with empty object"),
    }
}

#[test]
fn test_json_query_combined_wrapper_and_on_empty() {
    // Combine wrapper clause with ON EMPTY
    let json = json!({"items": []});

    // Empty array with EMPTY OBJECT ON EMPTY
    let result =
        eval_json_query_with_on_empty(json, "$.missing", "WITH WRAPPER", "EMPTY OBJECT").unwrap();
    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!({}));
        }
        _ => panic!("Expected empty object"),
    }
}

#[test]
fn test_json_query_all_parameters() {
    // Test with all 5 parameters
    let json = json!({"data": {"items": [{"id": 1}]}});

    // Valid path with all parameters
    let result = eval_json_query_full(
        json.clone(),
        "$.data.items",
        "WITH WRAPPER",
        "EMPTY ARRAY",
        "NULL",
    )
    .unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([[{"id": 1}]])); // Wrapped because of WITH WRAPPER
        }
        _ => panic!("Expected JsonB"),
    }

    // Missing path with EMPTY ARRAY ON EMPTY
    let result =
        eval_json_query_full(json, "$.missing", "WITHOUT WRAPPER", "EMPTY ARRAY", "NULL").unwrap();

    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([]));
        }
        _ => panic!("Expected empty array"),
    }
}

#[test]
fn test_json_query_on_empty_on_error_precedence() {
    // ON ERROR should take precedence over ON EMPTY when there's an error
    let json = json!({"name": "test"});

    // Invalid JSONPath - ON ERROR should handle it (not ON EMPTY)
    let result = eval_json_query_full(
        json,
        "$.[invalid",
        "WITHOUT WRAPPER",
        "ERROR",       // ON EMPTY = ERROR
        "EMPTY ARRAY", // ON ERROR = EMPTY ARRAY
    )
    .unwrap();

    // Should return empty array (from ON ERROR), not raise error (from ON EMPTY)
    match result {
        Literal::JsonB(value) => {
            assert_eq!(value, json!([]));
        }
        _ => panic!("Expected empty array from ON ERROR"),
    }
}
