// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Python (Starlark) wrapper code generator
//!
//! Generates the Starlark wrapper code that creates the `raisin` global
//! object with all the user-facing APIs using snake_case naming.

use crate::runtime::bindings::methods::registry;
use crate::runtime::bindings::registry::ReturnType;

/// Generate the Python/Starlark wrapper code
///
/// This creates the `raisin` global struct that exposes all APIs
/// in a Pythonic format with snake_case naming.
///
/// Note: Starlark uses `struct()` instead of classes/objects.
/// Functions are first-class values and can be stored in structs.
pub fn generate_python_wrapper() -> String {
    let reg = registry();
    let mut code = String::with_capacity(8192);

    code.push_str("# RaisinDB Python API Wrapper\n");
    code.push_str("# Auto-generated from shared bindings registry\n");
    code.push_str("# Uses Starlark struct() for object-like namespaces\n\n");

    // Generate individual function wrappers first
    code.push_str("# Internal wrapper functions\n\n");

    for method in reg.methods() {
        // Skip internal category for direct exposure
        if method.category == "internal" {
            continue;
        }
        code.push_str(&generate_python_function(method));
    }

    // Generate HTTP convenience functions
    code.push_str("# HTTP convenience functions\n");
    code.push_str("def _http_get(url, headers = {}, params = {}):\n");
    code.push_str(
        "    return _http_request(\"GET\", url, {\"headers\": headers, \"params\": params})\n\n",
    );
    code.push_str("def _http_post(url, json = None, headers = {}):\n");
    code.push_str(
        "    return _http_request(\"POST\", url, {\"headers\": headers, \"body\": json})\n\n",
    );
    code.push_str("def _http_put(url, json = None, headers = {}):\n");
    code.push_str(
        "    return _http_request(\"PUT\", url, {\"headers\": headers, \"body\": json})\n\n",
    );
    code.push_str("def _http_patch(url, json = None, headers = {}):\n");
    code.push_str(
        "    return _http_request(\"PATCH\", url, {\"headers\": headers, \"body\": json})\n\n",
    );
    code.push_str("def _http_delete(url, headers = {}):\n");
    code.push_str("    return _http_request(\"DELETE\", url, {\"headers\": headers})\n\n");

    // Build the main raisin struct
    code.push_str("# Build the raisin namespace\n");
    code.push_str("raisin = struct(\n");

    // Group methods by category
    let categories = reg.categories();

    for category in &categories {
        if *category == "internal" {
            continue;
        }
        if category.starts_with("admin_") {
            continue; // Handle admin separately
        }
        if *category == "notify" {
            continue; // Handle notify as direct method
        }

        let methods = reg.methods_by_category(category);
        if methods.is_empty() {
            continue;
        }

        // Special handling for http (uses convenience methods)
        if *category == "http" {
            code.push_str("    http = struct(\n");
            code.push_str("        request = _http_request,\n");
            code.push_str("        get = _http_get,\n");
            code.push_str("        post = _http_post,\n");
            code.push_str("        put = _http_put,\n");
            code.push_str("        patch = _http_patch,\n");
            code.push_str("        delete = _http_delete,\n");
            code.push_str("    ),\n");
            continue;
        }

        code.push_str(&format!("    {} = struct(\n", category));

        for method in &methods {
            code.push_str(&format!(
                "        {} = _{},\n",
                method.py_name, method.internal_name
            ));
        }

        code.push_str("    ),\n");
    }

    // Generate notify as a direct method
    let notify_methods = reg.methods_by_category("notify");
    for method in &notify_methods {
        code.push_str(&format!(
            "    {} = _{},\n",
            method.py_name, method.internal_name
        ));
    }

    // Generate admin namespace
    let admin_nodes = reg.methods_by_category("admin_nodes");
    let admin_sql = reg.methods_by_category("admin_sql");
    if !admin_nodes.is_empty() || !admin_sql.is_empty() {
        code.push_str("    admin = struct(\n");

        if !admin_nodes.is_empty() {
            code.push_str("        nodes = struct(\n");
            for method in &admin_nodes {
                code.push_str(&format!(
                    "            {} = _{},\n",
                    method.py_name, method.internal_name
                ));
            }
            code.push_str("        ),\n");
        }

        if !admin_sql.is_empty() {
            code.push_str("        sql = struct(\n");
            for method in &admin_sql {
                code.push_str(&format!(
                    "            {} = _{},\n",
                    method.py_name, method.internal_name
                ));
            }
            code.push_str("        ),\n");
        }

        code.push_str("    ),\n");
    }

    // Context (note: in Starlark this is a function call, not a property)
    code.push_str("    context = _context_get(),\n");

    code.push_str(")\n");

    code
}

/// Generate a single Python function wrapper
fn generate_python_function(
    method: &crate::runtime::bindings::registry::ApiMethodDescriptor,
) -> String {
    let mut code = String::new();

    // Build argument list with defaults for optional args
    let args: Vec<String> = method
        .args
        .iter()
        .map(|a| match a.arg_type {
            crate::runtime::bindings::registry::ArgType::OptionalString
            | crate::runtime::bindings::registry::ArgType::OptionalJson
            | crate::runtime::bindings::registry::ArgType::OptionalU32
            | crate::runtime::bindings::registry::ArgType::OptionalI64
            | crate::runtime::bindings::registry::ArgType::OptionalBool => {
                format!("{} = None", a.name)
            }
            crate::runtime::bindings::registry::ArgType::JsonArray => {
                format!("{} = []", a.name)
            }
            _ => a.name.to_string(),
        })
        .collect();
    let args_str = args.join(", ");

    // Function definition
    code.push_str(&format!("def _{}({}):\n", method.internal_name, args_str));

    // Build internal call
    let call_args: Vec<&str> = method.args.iter().map(|a| a.name).collect();
    let call_args_str = call_args.join(", ");

    // Note: In actual implementation, these would call the internal bindings
    // For now, generate pseudocode showing the call pattern
    code.push_str(&format!(
        "    # Calls internal: __internal_{}({})\n",
        method.internal_name, call_args_str
    ));

    // Return based on return type
    match method.return_type {
        ReturnType::Void => {
            code.push_str(&format!(
                "    return __internal_{}({})\n",
                method.internal_name, call_args_str
            ));
        }
        _ => {
            code.push_str(&format!(
                "    return __internal_{}({})\n",
                method.internal_name, call_args_str
            ));
        }
    }

    code.push('\n');
    code
}

/// Get the generated wrapper code as a static string (cached)
pub fn get_wrapper_code() -> &'static str {
    use std::sync::OnceLock;
    static WRAPPER: OnceLock<String> = OnceLock::new();
    WRAPPER.get_or_init(generate_python_wrapper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_wrapper() {
        let code = generate_python_wrapper();

        // Should have basic structure
        assert!(code.contains("raisin = struct("));
        assert!(code.contains("nodes = struct("));
        assert!(code.contains("sql = struct("));
        assert!(code.contains("ai = struct("));

        // Should have HTTP convenience functions
        assert!(code.contains("def _http_get"));
        assert!(code.contains("def _http_post"));

        // Should have notify as a direct method
        assert!(code.contains("notify = _notify_send"));
    }

    #[test]
    fn test_snake_case_names() {
        let code = generate_python_wrapper();

        // Should use snake_case for Python method names
        assert!(code.contains("get_by_id"));
        assert!(code.contains("get_children"));
        assert!(code.contains("update_property"));
        assert!(code.contains("list_models"));
        assert!(code.contains("get_default_model"));
    }

    #[test]
    fn test_admin_namespace() {
        let code = generate_python_wrapper();

        // Should have admin namespace
        assert!(code.contains("admin = struct("));
    }
}
