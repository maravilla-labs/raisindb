// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Starlark Python-like runtime implementation
//!
//! This module provides a Python-like runtime using the Starlark crate.
//! Starlark is a Python dialect designed for configuration, originally
//! developed by Google for Bazel.

mod conversions;
mod gateway;
mod runtime_impl;
mod setup_code;
mod thread_local;

pub use runtime_impl::StarlarkRuntime;

#[cfg(test)]
mod tests {
    use super::conversions::{json_to_starlark, starlark_value_to_json};
    use super::*;
    use crate::api::MockFunctionApi;
    use crate::runtime::FunctionRuntime;
    use crate::types::{ExecutionContext, FunctionMetadata};
    use starlark::environment::Module;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_test_context() -> ExecutionContext {
        ExecutionContext::new("test-tenant", "test-repo", "main", "test-user")
            .with_workspace("default")
            .with_input(serde_json::json!({"value": 42}))
    }

    #[test]
    fn test_validate_empty_code() {
        let runtime = StarlarkRuntime::new();
        let result = runtime.validate("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_no_def() {
        let runtime = StarlarkRuntime::new();
        let result = runtime.validate("x = 1 + 2");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_valid_code() {
        let runtime = StarlarkRuntime::new();
        let result = runtime.validate(
            r#"
def handler(input):
    return input
"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_syntax_error() {
        let runtime = StarlarkRuntime::new();
        let result = runtime.validate(
            r#"
def handler(input)
    return input  # Missing colon
"#,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_simple_function() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({
            "tenant_id": "test-tenant",
            "repo_id": "test-repo",
            "branch": "main"
        })));

        let code = r#"
def handler(input):
    return "hello"
"#;

        let context = create_test_context();
        let metadata = FunctionMetadata::starlark("test-function");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await;

        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert!(exec_result.success);
        assert_eq!(exec_result.output, Some(serde_json::json!("hello")));
    }

    #[tokio::test]
    async fn test_execute_with_context() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({
            "tenant_id": "test-tenant",
            "repo_id": "test-repo",
            "branch": "main"
        })));

        let code = r#"
def handler(input):
    return raisin.context.tenant_id
"#;

        let context = create_test_context();
        let metadata = FunctionMetadata::starlark("test-function");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await;

        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert!(exec_result.success);
        assert_eq!(exec_result.output, Some(serde_json::json!("test-tenant")));
    }

    #[tokio::test]
    async fn test_execute_missing_handler() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

        let code = r#"
def other_function():
    return "hello"
"#;

        let context = create_test_context();
        let metadata = FunctionMetadata::starlark("test-function");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_with_load_module() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({
            "tenant_id": "test-tenant",
            "repo_id": "test-repo",
            "branch": "main"
        })));

        let code = r#"
load("math.star", "plus")

def handler(input):
    return plus(40, 2)
"#;

        let mut files = HashMap::new();
        files.insert(
            "math.star".to_string(),
            r#"
def plus(a, b):
    return a + b
"#
            .to_string(),
        );

        let context = create_test_context();
        let metadata =
            FunctionMetadata::starlark("test-function").with_entry_file("index.star:handler");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, files)
            .await;

        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert!(exec_result.success);
        assert_eq!(exec_result.output, Some(serde_json::json!(42)));
    }

    #[tokio::test]
    async fn test_execute_with_relative_load_module() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({
            "tenant_id": "test-tenant",
            "repo_id": "test-repo",
            "branch": "main"
        })));

        let code = r#"
load("./utils.star", "mul")

def handler(input):
    return mul(6, 7)
"#;

        let mut files = HashMap::new();
        files.insert(
            "src/utils.star".to_string(),
            r#"
def mul(a, b):
    return a * b
"#
            .to_string(),
        );

        let context = create_test_context();
        let metadata =
            FunctionMetadata::starlark("test-function").with_entry_file("src/main.star:handler");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, files)
            .await;

        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert!(exec_result.success);
        assert_eq!(exec_result.output, Some(serde_json::json!(42)));
    }

    #[test]
    fn test_starlark_value_to_json() {
        let module = Module::new();
        let heap = module.heap();

        // Test string
        let str_val = heap.alloc("hello");
        assert_eq!(starlark_value_to_json(str_val), serde_json::json!("hello"));

        // Test integer
        let int_val = heap.alloc(42);
        assert_eq!(starlark_value_to_json(int_val), serde_json::json!(42));

        // Test bool
        let bool_val = heap.alloc(true);
        assert_eq!(starlark_value_to_json(bool_val), serde_json::json!(true));
    }

    #[test]
    fn test_json_to_starlark() {
        let module = Module::new();
        let heap = module.heap();

        // Test string
        let json_str = serde_json::json!("hello");
        let val = json_to_starlark(heap, &json_str);
        assert_eq!(val.unpack_str(), Some("hello"));

        // Test number
        let json_num = serde_json::json!(42);
        let val = json_to_starlark(heap, &json_num);
        assert_eq!(val.unpack_i32(), Some(42));

        // Test bool
        let json_bool = serde_json::json!(true);
        let val = json_to_starlark(heap, &json_bool);
        assert_eq!(val.unpack_bool(), Some(true));
    }
}
