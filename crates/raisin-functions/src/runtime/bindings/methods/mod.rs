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
pub mod assets;
pub mod branches;
pub mod context;
pub mod crypto;
pub mod date;
pub mod email;
pub mod events;
pub mod flows;
pub mod functions;
pub mod http;
pub mod identities;
pub mod imap;
pub mod integrations;
pub mod locks;
pub mod nodes;
pub mod notify;
pub mod ocr;
pub mod pdf;
pub mod platform;
pub mod resources;
pub mod scheduler;
pub mod secrets;
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
    methods.extend(flows::methods());
    methods.extend(branches::methods());
    methods.extend(scheduler::methods());
    methods.extend(platform::methods());
    methods.extend(notify::methods());
    methods.extend(locks::methods());
    methods.extend(integrations::methods());
    methods.extend(imap::methods());
    methods.extend(email::methods());
    methods.extend(secrets::methods());
    methods.extend(identities::methods());

    // Resource operations
    methods.extend(resources::methods());
    methods.extend(ocr::methods());
    methods.extend(pdf::methods());
    // Extraction writeback — text produced by a plugin task above this layer.
    methods.extend(assets::methods());

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
            // AI operations (5)
            "ai_completion",
            "ai_list_models",
            "ai_list_providers",
            "ai_get_default_model",
            "ai_embed",
            // Resource operations (2)
            "resource_get_binary",
            "node_add_resource",
            // PDF (1)
            "pdf_process_from_storage",
            // Asset operations (1)
            "asset_set_extraction",
            "asset_reextract",
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
            // Lock / inventory operations (5)
            "lock_acquire",
            "lock_release",
            "lock_renew",
            "inventory_claim",
            "inventory_release",
            // Secret operations (6)
            "secret_get",
            "secret_resolve",
            "secret_put",
            "secret_list",
            "secret_rotate",
            "secret_delete",
            // Integration / mount operations (1)
            "integrations_sync_now",
            // IMAP operations (3)
            "imap_fetch_since",
            "imap_list_mailboxes",
            "imap_fetch_message",
            // Email operations (2)
            "email_send",
            "email_providers",
            // Crypto operations (1)
            "crypto_verify_jwt",
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
            .replace("locks_", "lock_")
            .replace("secrets_", "secret_")
            .replace("identities_", "identity_")
            .replace("events_", "event_")
            .replace("tasks_", "task_")
            .replace("functions_", "function_")
            .replace("resources_", "resource_")
            // CamelCase to snake_case
            .replace("getById", "get_by_id")
            .replace("findByEmail", "find_by_email")
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
            .replace("listProviders", "list_providers")
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
        // plus 5 lock/inventory methods and 2 branch methods (diff/compare).
        // This test helps catch accidental duplicate registrations or missing methods
        assert!(
            method_count >= 58,
            "Expected at least 58 methods, got {}. Did you forget to add some bindings?",
            method_count
        );

        // Upper bound check (shouldn't have too many extra methods).
        // This is a duplicate-registration tripwire, not a budget: raise it
        // deliberately when the surface genuinely grows. It was already blown
        // (101 > 100) before the crypto signing bindings were added.
        assert!(
            method_count <= 130,
            "Expected at most 130 methods, got {}. Did you accidentally duplicate some bindings?",
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

    /// The QuickJS surface (`quickjs/api_wrapper.js`) dispatches every
    /// `raisin.*` method through `__raisin_call('<internal_name>', [...])`.
    /// This test pins the wrapper to the registry in BOTH directions:
    ///
    /// 1. every method the wrapper references must exist in the registry
    ///    (a typo'd or removed method fails here, not at function runtime);
    /// 2. every registry method must either be referenced by the wrapper or
    ///    be on the explicit not-exposed list — so adding a registry method
    ///    forces a conscious decision about the JS surface.
    #[test]
    fn test_quickjs_wrapper_matches_registry() {
        // Registry methods deliberately NOT exposed on the JS surface.
        const NOT_EXPOSED_IN_JS: &[(&str, &str)] = &[
            (
                "tx_move",
                "intentionally unsupported in JS transactions (see api_wrapper.js)",
            ),
            (
                "context_get",
                "raisin.context is injected as a data property by environment.rs",
            ),
            ("log", "JS uses the console host functions"),
            ("date_now", "JS uses the native Date object"),
            ("date_timestamp", "JS uses the native Date object"),
            ("date_timestampMillis", "JS uses the native Date object"),
            ("date_parse", "JS uses the native Date object"),
            ("date_format", "JS uses the native Date object"),
            ("date_addDays", "JS uses the native Date object"),
            ("date_diffDays", "JS uses the native Date object"),
            (
                "tasks_update",
                "not part of the frozen JS surface (create + complete only)",
            ),
            (
                "tasks_query",
                "not part of the frozen JS surface (create + complete only)",
            ),
        ];

        let wrapper = include_str!("../../quickjs/api_wrapper.js");

        // Collect every literal __call('name', ...) reference.
        let mut referenced = std::collections::HashSet::new();
        for (idx, _) in wrapper.match_indices("__call(") {
            let rest = &wrapper[idx + "__call(".len()..];
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('\'') else {
                panic!(
                    "__call must be invoked with a literal single-quoted method name \
                     (offset {} in api_wrapper.js)",
                    idx
                );
            };
            let name = rest
                .split('\'')
                .next()
                .expect("unterminated method name in __call(...)");
            referenced.insert(name.to_string());
        }
        assert!(
            !referenced.is_empty(),
            "api_wrapper.js should dispatch through __call(...)"
        );

        let reg = build_registry();
        let registry_names: std::collections::HashSet<&str> =
            reg.methods().iter().map(|m| m.internal_name).collect();

        // 1. Wrapper references only registered methods.
        for name in &referenced {
            assert!(
                registry_names.contains(name.as_str()),
                "api_wrapper.js calls '{}' which is not in the bindings registry",
                name
            );
        }

        // 2. Registry methods are either exposed or explicitly excluded.
        let excluded: std::collections::HashSet<&str> =
            NOT_EXPOSED_IN_JS.iter().map(|(n, _)| *n).collect();
        for method in reg.methods() {
            let name = method.internal_name;
            assert!(
                referenced.contains(name) || excluded.contains(name),
                "Registry method '{}' is neither referenced by api_wrapper.js nor on the \
                 NOT_EXPOSED_IN_JS list. Expose it in the wrapper (with an explicit error \
                 convention) or add it to the list with a reason.",
                name
            );
        }

        // The exclusion list must not contain stale entries.
        for (name, _) in NOT_EXPOSED_IN_JS {
            assert!(
                registry_names.contains(name),
                "NOT_EXPOSED_IN_JS lists '{}' which is not in the registry",
                name
            );
            assert!(
                !referenced.contains(*name),
                "NOT_EXPOSED_IN_JS lists '{}' but api_wrapper.js references it",
                name
            );
        }
    }

    /// `raisin.notify` must be reachable from JavaScript functions. It used to
    /// exist only in the Rust registry (as `notify_send`) and on the Starlark
    /// surface, but the QuickJS wrapper never exposed it, so every JS call to
    /// `raisin.notify(...)` threw "not a function". Guard the exposure here so
    /// it cannot silently regress.
    #[test]
    fn test_notify_exposed_in_js() {
        let wrapper = include_str!("../../quickjs/api_wrapper.js");
        assert!(
            wrapper.contains("__call('notify_send'"),
            "api_wrapper.js must dispatch raisin.notify through __call('notify_send', ...)"
        );

        // And notify_send must be a real, registered binding.
        let reg = build_registry();
        assert!(
            reg.methods()
                .iter()
                .any(|m| m.internal_name == "notify_send"),
            "notify_send must exist in the bindings registry"
        );

        // raisin.tasks.create is the other framework-task surface JS relies on.
        assert!(
            wrapper.contains("__call('tasks_create'"),
            "api_wrapper.js must expose raisin.tasks.create"
        );

        // raisin.tasks.complete must be reachable from JS: it marks a task
        // completed AND resumes the owning flow (the P0 binding). Guard the
        // exposure so it cannot silently regress back off the frozen surface.
        assert!(
            wrapper.contains("__call('tasks_complete'"),
            "api_wrapper.js must expose raisin.tasks.complete"
        );
        assert!(
            reg.methods()
                .iter()
                .any(|m| m.internal_name == "tasks_complete"),
            "tasks_complete must exist in the bindings registry"
        );
    }
}
