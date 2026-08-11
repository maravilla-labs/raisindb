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

    /// Parity with the QuickJS `test_locks_and_inventory_bindings`: `raisin.locks.*`
    /// and `raisin.inventory.*` are exposed via the shared registry and behave
    /// identically (guard-or-None, {remaining}-or-None).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_locks_and_inventory_bindings() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

        let code = r#"
def handler(input):
    first = raisin.inventory.claim("flight:1", 1, 2)
    second = raisin.inventory.claim("flight:1", 1, 2)
    sold_out = raisin.inventory.claim("flight:1", 1, 2)
    lock = raisin.locks.acquire("seat:14A", 5000)
    contended = raisin.locks.acquire("seat:14A", 5000)
    released = raisin.locks.release("seat:14A", lock["token"])
    return {
        "firstClaimed": first["claimed"],
        "firstRemaining": first["remaining"],
        "secondRemaining": second["remaining"],
        "soldOut": sold_out["claimed"],
        "acquired": lock["acquired"],
        "acquiredToken": lock["token"],
        "contended": contended["acquired"],
        "released": released,
    }
"#;

        let context = create_test_context();
        let metadata = FunctionMetadata::starlark("test-locks");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await
            .expect("execution ok");
        assert!(result.success, "function errored: {:?}", result.error);
        let output = result.output.unwrap();
        assert_eq!(output["firstClaimed"], true);
        assert_eq!(output["firstRemaining"], 1);
        assert_eq!(output["secondRemaining"], 0);
        assert_eq!(output["soldOut"], false); // sold out → claimed:false
        assert_eq!(output["acquired"], true);
        assert!(output["acquiredToken"].as_i64().unwrap() > 0);
        assert_eq!(output["contended"], false); // contended → acquired:false
        assert_eq!(output["released"], true);
    }

    /// Parity with the QuickJS `test_secrets_bindings`: `raisin.secrets.*` is
    /// exposed via the shared registry (Starlark auto-generates the namespace
    /// from the descriptor `category`) and behaves identically — same names,
    /// same argument order, same append-not-overwrite semantics.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_secrets_bindings() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

        let code = r#"
def handler(input):
    first = raisin.secrets.put("stripe/live", "sk_v1")
    read_back = raisin.secrets.get("stripe/live")
    rotated = raisin.secrets.rotate("stripe/live", "sk_v2")
    after_rotate = raisin.secrets.get("stripe/live")
    pinned = raisin.secrets.get("stripe/live", 1)
    raisin.secrets.put("sendgrid", "sg_v1")
    listed = raisin.secrets.list()
    tomb = raisin.secrets.delete("sendgrid")
    return {
        "firstVersion": first["version"],
        "firstName": first["name"],
        "readBack": read_back,
        "rotatedVersion": rotated["version"],
        "afterRotate": after_rotate,
        "pinned": pinned,
        "listedNames": [s["name"] for s in listed],
        "tombVersion": tomb["version"],
    }
"#;

        let context = create_test_context();
        let metadata = FunctionMetadata::starlark("test-secrets");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await
            .expect("execution ok");
        assert!(result.success, "function errored: {:?}", result.error);
        let output = result.output.unwrap();
        assert_eq!(output["firstVersion"], 1);
        assert_eq!(output["firstName"], "stripe/live");
        assert_eq!(output["readBack"], "sk_v1");
        assert_eq!(output["rotatedVersion"], 2);
        assert_eq!(output["afterRotate"], "sk_v2");
        assert_eq!(output["pinned"], "sk_v1");
        assert_eq!(
            output["listedNames"],
            serde_json::json!(["sendgrid", "stripe/live"])
        );
        assert_eq!(output["tombVersion"], 2);
    }

    /// Parity with the QuickJS `test_secrets_accepts_references`: the reference
    /// spelling, the honoured pin and `resolve`'s passthrough behave identically
    /// in Starlark.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_secrets_accepts_references() {
        let runtime = StarlarkRuntime::new();
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

        let code = r#"
def handler(input):
    raisin.secrets.put("node/01H8XY/api_key", "v1")
    raisin.secrets.rotate("node/01H8XY/api_key", "v2")
    property_value = "secret://node/01H8XY/api_key@1"
    return {
        "pinned": raisin.secrets.get(property_value),
        "unpinned": raisin.secrets.get("secret://node/01H8XY/api_key"),
        "bareName": raisin.secrets.get("node/01H8XY/api_key"),
        "resolvedRef": raisin.secrets.resolve(property_value),
        "resolvedLiteral": raisin.secrets.resolve("a-plain-password"),
    }
"#;

        let context = create_test_context();
        let metadata = FunctionMetadata::starlark("test-secret-refs");

        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await
            .expect("execution ok");
        assert!(result.success, "function errored: {:?}", result.error);
        let output = result.output.unwrap();
        assert_eq!(output["pinned"], "v1");
        assert_eq!(output["unpinned"], "v2");
        assert_eq!(output["bareName"], "v2");
        assert_eq!(output["resolvedRef"], "v1");
        assert_eq!(output["resolvedLiteral"], "a-plain-password");
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
