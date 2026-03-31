// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Method definitions organized by category
//!
//! Each submodule defines methods for a specific API category.
//! All methods are combined into a single registry for use by runtime adapters.

pub mod admin;
pub mod ai;
pub mod context;
pub mod crypto;
pub mod date;
pub mod events;
pub mod functions;
pub mod http;
pub mod nodes;
pub mod notify;
pub mod pdf;
pub mod resources;
pub mod sql;
pub mod tasks;
pub mod transactions;

use super::registry::{ApiMethodDescriptor, BindingsRegistry};

/// Build the complete bindings registry with all methods
pub fn build_registry() -> BindingsRegistry {
    let mut methods = Vec::new();

    // Core operations
    methods.extend(nodes::methods());
    methods.extend(sql::methods());
    methods.extend(http::methods());
    methods.extend(ai::methods());
    methods.extend(events::methods());
    methods.extend(tasks::methods());
    methods.extend(functions::methods());
    methods.extend(notify::methods());

    // Resource operations
    methods.extend(resources::methods());
    methods.extend(pdf::methods());

    // Date/time operations
    methods.extend(date::methods());

    // Crypto operations
    methods.extend(crypto::methods());

    // Transaction operations
    methods.extend(transactions::methods());

    // Admin operations (RLS bypass)
    methods.extend(admin::methods());

    // Context and logging
    methods.extend(context::methods());

    BindingsRegistry::with_methods(methods)
}

/// Get the global bindings registry (lazily initialized)
pub fn registry() -> &'static BindingsRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<BindingsRegistry> = OnceLock::new();
    REGISTRY.get_or_init(build_registry)
}

/// Get all method descriptors
pub fn all_methods() -> &'static [ApiMethodDescriptor] {
    registry().methods()
}

/// Get methods for a specific category
pub fn methods_by_category(category: &str) -> Vec<&'static ApiMethodDescriptor> {
    registry().methods_by_category(category)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_methods() {
        let reg = build_registry();
        assert!(!reg.methods().is_empty(), "Registry should have methods");

        // Check we have all expected categories
        let categories = reg.categories();
        assert!(categories.contains(&"nodes"), "Should have nodes category");
        assert!(categories.contains(&"sql"), "Should have sql category");
        assert!(categories.contains(&"http"), "Should have http category");
        assert!(categories.contains(&"ai"), "Should have ai category");
        assert!(
            categories.contains(&"events"),
            "Should have events category"
        );
        assert!(categories.contains(&"tasks"), "Should have tasks category");
        assert!(
            categories.contains(&"functions"),
            "Should have functions category"
        );
        assert!(categories.contains(&"tx"), "Should have tx category");
        assert!(
            categories.contains(&"admin_nodes"),
            "Should have admin_nodes category"
        );
        assert!(
            categories.contains(&"admin_sql"),
            "Should have admin_sql category"
        );
        assert!(
            categories.contains(&"resources"),
            "Should have resources category"
        );
        assert!(categories.contains(&"pdf"), "Should have pdf category");
        assert!(
            categories.contains(&"context"),
            "Should have context category"
        );
        assert!(categories.contains(&"date"), "Should have date category");
        assert!(
            categories.contains(&"notify"),
            "Should have notify category"
        );
        assert!(
            categories.contains(&"crypto"),
            "Should have crypto category"
        );
    }

    #[test]
    fn test_node_methods() {
        let methods = methods_by_category("nodes");
        assert!(methods.len() >= 9, "Should have at least 9 node methods");

        let names: Vec<&str> = methods.iter().map(|m| m.internal_name).collect();
        assert!(names.contains(&"nodes_get"), "Should have nodes_get");
        assert!(
            names.contains(&"nodes_getById"),
            "Should have nodes_getById"
        );
        assert!(names.contains(&"nodes_create"), "Should have nodes_create");
        assert!(names.contains(&"nodes_update"), "Should have nodes_update");
        assert!(names.contains(&"nodes_delete"), "Should have nodes_delete");
        assert!(names.contains(&"nodes_query"), "Should have nodes_query");
        assert!(
            names.contains(&"nodes_getChildren"),
            "Should have nodes_getChildren"
        );
    }

    #[test]
    fn test_transaction_methods() {
        let methods = methods_by_category("tx");
        assert!(methods.len() >= 18, "Should have at least 18 tx methods");

        let names: Vec<&str> = methods.iter().map(|m| m.internal_name).collect();
        assert!(names.contains(&"tx_begin"), "Should have tx_begin");
        assert!(names.contains(&"tx_commit"), "Should have tx_commit");
        assert!(names.contains(&"tx_rollback"), "Should have tx_rollback");
        assert!(names.contains(&"tx_create"), "Should have tx_create");
    }

    #[test]
    fn test_python_names_are_snake_case() {
        let reg = build_registry();
        for method in reg.methods() {
            // Check that Python names don't contain uppercase letters
            // (except for single-letter words which are allowed)
            let has_consecutive_uppercase = method
                .py_name
                .chars()
                .zip(method.py_name.chars().skip(1))
                .any(|(a, b)| a.is_ascii_uppercase() && b.is_ascii_lowercase());

            assert!(
                !has_consecutive_uppercase,
                "Python name '{}' should be snake_case, not camelCase",
                method.py_name
            );
        }
    }

    /// This test verifies that all FunctionApi trait methods are bound in the registry.
    /// If this test fails, it means you added a new method to FunctionApi but didn't
    /// add a corresponding binding in the shared bindings layer.
    #[test]
    fn test_api_parity_all_function_api_methods_bound() {
        // Complete list of FunctionApi trait methods
        // This must be manually updated when new methods are added to FunctionApi
        let function_api_methods = [
            // Node operations (9)
            "node_get",
            "node_get_by_id",
            "node_create",
            "node_update",
            "node_delete",
            "node_update_property",
            "node_move",
            "node_query",
            "node_get_children",
            // SQL operations (2)
            "sql_query",
            "sql_execute",
            // HTTP (1)
            "http_request",
            // Events (1)
            "emit_event",
            // AI operations (4)
            "ai_completion",
            "ai_list_models",
            "ai_get_default_model",
            "ai_embed",
            // Resource operations (2)
            "resource_get_binary",
            "node_add_resource",
            // PDF (1)
            "pdf_process_from_storage",
            // Tasks (1)
            "task_create",
            // Functions (1)
            "function_execute",
            // Transaction operations (19)
            "tx_begin",
            "tx_commit",
            "tx_rollback",
            "tx_set_actor",
            "tx_set_message",
            "tx_create",
            "tx_add",
            "tx_put",
            "tx_upsert",
            "tx_create_deep",
            "tx_upsert_deep",
            "tx_update",
            "tx_delete",
            "tx_delete_by_id",
            "tx_get",
            "tx_get_by_path",
            "tx_list_children",
            "tx_move",
            "tx_update_property",
            // Context and logging (3)
            "log",
            "get_context",
            "allows_admin_escalation",
            // Admin node operations (8)
            "admin_node_get",
            "admin_node_get_by_id",
            "admin_node_create",
            "admin_node_update",
            "admin_node_delete",
            "admin_node_update_property",
            "admin_node_query",
            "admin_node_get_children",
            // Admin SQL operations (2)
            "admin_sql_query",
            "admin_sql_execute",
            // Date/time operations (7)
            "date_now",
            "date_timestamp",
            "date_timestamp_millis",
            "date_parse",
            "date_format",
            "date_add_days",
            "date_diff_days",
        ];

        let reg = build_registry();
        let bound_names: Vec<&str> = reg.methods().iter().map(|m| m.internal_name).collect();

        // Check that each FunctionApi method has a corresponding binding
        let mut missing = Vec::new();
        for api_method in function_api_methods.iter() {
            // Internal names use different conventions (e.g., nodes_get instead of node_get)
            // So we normalize for comparison
            let normalized = normalize_method_name(api_method);
            if !bound_names
                .iter()
                .any(|n| normalize_method_name(n) == normalized)
            {
                missing.push(*api_method);
            }
        }

        assert!(
            missing.is_empty(),
            "The following FunctionApi methods are NOT bound in the shared registry:\n  - {}\n\n\
            To fix this, add bindings for these methods in the appropriate methods/*.rs file.\n\
            Total bound: {}, Total expected: {}",
            missing.join("\n  - "),
            bound_names.len(),
            function_api_methods.len()
        );
    }

    /// Helper to normalize method names for comparison
    fn normalize_method_name(name: &str) -> String {
        // Convert variations to a canonical form:
        // - nodes_get -> node_get
        // - nodes_getById -> node_get_by_id
        // - events_emit -> emit_event
        // - tasks_create -> task_create
        // - functions_execute -> function_execute
        // - resources_getBinary -> resource_get_binary
        let normalized = name
            // Plurals to singular for category prefixes
            .replace("nodes_", "node_")
            .replace("events_", "event_")
            .replace("tasks_", "task_")
            .replace("functions_", "function_")
            .replace("resources_", "resource_")
            // CamelCase to snake_case
            .replace("getById", "get_by_id")
            .replace("getChildren", "get_children")
            .replace("updateProperty", "update_property")
            .replace("deleteById", "delete_by_id")
            .replace("getByPath", "get_by_path")
            .replace("listChildren", "list_children")
            .replace("setActor", "set_actor")
            .replace("setMessage", "set_message")
            .replace("createDeep", "create_deep")
            .replace("upsertDeep", "upsert_deep")
            .replace("getBinary", "get_binary")
            .replace("addResource", "add_resource")
            .replace("processFromStorage", "process_from_storage")
            .replace("listModels", "list_models")
            .replace("getDefaultModel", "get_default_model")
            .replace("allowsAdminEscalation", "allows_admin_escalation")
            .replace("emitEvent", "emit_event")
            .replace("context_get", "get_context")
            // Date method conversions
            .replace("timestampMillis", "timestamp_millis")
            .replace("addDays", "add_days")
            .replace("diffDays", "diff_days")
            .to_lowercase();

        // Handle special cases where order matters:
        // event_emit -> emit_event (swap action and category)
        if normalized == "event_emit" {
            return "emit_event".to_string();
        }

        normalized
    }

    #[test]
    fn test_method_count_matches_expected() {
        let reg = build_registry();
        let method_count = reg.methods().len();

        // Expected: 62 methods from FunctionApi (54 original + 7 date methods + 1 notify method)
        // This test helps catch accidental duplicate registrations or missing methods
        assert!(
            method_count >= 58,
            "Expected at least 58 methods, got {}. Did you forget to add some bindings?",
            method_count
        );

        // Upper bound check (shouldn't have too many extra methods)
        assert!(
            method_count <= 68,
            "Expected at most 68 methods, got {}. Did you accidentally duplicate some bindings?",
            method_count
        );

        println!(
            "Registry has {} methods across {} categories",
            method_count,
            reg.categories().len()
        );
    }

    #[test]
    fn test_all_methods_have_unique_internal_names() {
        let reg = build_registry();
        let mut seen = std::collections::HashSet::new();

        for method in reg.methods() {
            assert!(
                seen.insert(method.internal_name),
                "Duplicate internal name: '{}'. Each method must have a unique internal name.",
                method.internal_name
            );
        }
    }

    #[test]
    fn test_javascript_and_python_wrappers_cover_all_methods() {
        use crate::runtime::bindings::wrappers::javascript::get_wrapper_code as js_code;
        use crate::runtime::bindings::wrappers::python::get_wrapper_code as py_code;

        let js = js_code();
        let py = py_code();

        // JS wrapper should have the raisin object
        assert!(
            js.contains("globalThis.raisin"),
            "JS wrapper should define globalThis.raisin"
        );

        // Python wrapper should have the raisin struct
        assert!(
            py.contains("raisin = struct("),
            "Python wrapper should define raisin struct"
        );

        let reg = build_registry();

        // Check that all non-internal methods appear in the wrappers
        for method in reg.methods() {
            if method.category == "internal" {
                continue;
            }

            // JavaScript wrapper should contain the JS method name
            assert!(
                js.contains(method.js_name) || js.contains(&format!("{}:", method.js_name)),
                "JS wrapper missing method '{}' (internal: {})",
                method.js_name,
                method.internal_name
            );

            // Python wrapper should contain the Python method name
            // (checking for = _internal pattern)
            let py_binding = format!("{} = _", method.py_name);
            let py_binding_alt = format!("{}=_", method.py_name);
            assert!(
                py.contains(&py_binding)
                    || py.contains(&py_binding_alt)
                    || py.contains(method.py_name),
                "Python wrapper missing method '{}' (internal: {})",
                method.py_name,
                method.internal_name
            );
        }
    }
}
