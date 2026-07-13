// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared bindings layer for function runtimes
//!
//! This module provides a unified binding registry that both QuickJS and Starlark
//! runtimes use. API methods are defined ONCE here, and both runtimes generate
//! their bindings from this single source.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    User Code (JS / Python)                   │
//! ├─────────────────────────────────────────────────────────────┤
//! │  QuickJS Runtime            │      Starlark Runtime          │
//! │  quickjs/gateway.rs         │      starlark/gateway.rs       │
//! │  (__raisin_call host fn)    │      (__raisin_call builtin)   │
//! │  + quickjs/api_wrapper.js   │      + starlark/setup_code.rs  │
//! ├─────────────────────────────┴───────────────────────────────┤
//! │              SHARED BINDINGS REGISTRY                        │
//! │   - Every API method defined ONCE (methods/*)               │
//! │   - Declarative definitions per category                    │
//! │   - Test-time verification of completeness                  │
//! ├─────────────────────────────────────────────────────────────┤
//! │                    FunctionApi Trait                         │
//! │              (Arc<dyn FunctionApi>)                          │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Benefits
//!
//! - **Single Definition**: Each API method defined ONCE
//! - **Test-Time Verification**: Tests verify all FunctionApi methods are bound
//!   and that the QuickJS wrapper only references registered methods
//! - **Same Invoker**: Both runtimes call identical Rust async functions
//!
//! Each runtime keeps a thin surface layer on top of its gateway (Starlark: a
//! generated `raisin` struct that fails on any error envelope; QuickJS:
//! `api_wrapper.js` with the frozen per-method error conventions the JS
//! ecosystem depends on).
//!
//! ## Categories
//!
//! - `nodes` - Node CRUD operations
//! - `sql` - SQL query/execute
//! - `http` - HTTP requests
//! - `ai` - AI completion, embedding, model listing
//! - `events` - Event emission
//! - `tasks` - Task creation
//! - `functions` - Function execution
//! - `resources` - Binary resource access
//! - `pdf` - PDF processing
//! - `tx` - Transaction operations
//! - `admin_nodes` - Admin node operations (bypass RLS)
//! - `admin_sql` - Admin SQL operations (bypass RLS)
//! - `context` - Execution context
//! - `internal` - Logging and internal helpers

pub mod methods;
pub mod registry;

// Re-export key types
pub use methods::{all_methods, build_registry, methods_by_category, registry};
pub use registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, BindingsRegistry, InvokeResult, InvokerFn,
    ReturnType,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that we have bindings for the expected number of FunctionApi methods
    #[test]
    fn test_binding_count() {
        let reg = registry();
        let count = reg.methods().len();

        // We expect approximately 55+ methods:
        // - nodes: 11 (get, getById, create, update, delete, updateProperty, move, query, getChildren, applyChildOrder, addResource)
        // - sql: 2 (query, execute)
        // - http: 1 (request)
        // - ai: 4 (completion, listModels, getDefaultModel, embed)
        // - events: 1 (emit)
        // - tasks: 1 (create)
        // - functions: 1 (execute)
        // - resources: 1 (getBinary)
        // - pdf: 1 (processFromStorage)
        // - tx: 19 (begin, commit, rollback, setActor, setMessage, create, add, put, upsert,
        //          createDeep, upsertDeep, update, delete, deleteById, get, getByPath, listChildren, move, updateProperty)
        // - admin_nodes: 8
        // - admin_sql: 2
        // - context/internal: 3 (get, log, allowsAdminEscalation)
        //
        // Total: ~54 methods

        assert!(count >= 50, "Expected at least 50 methods, got {}", count);
        println!("Total binding count: {}", count);
    }

    /// List all methods for documentation
    #[test]
    fn test_list_all_methods() {
        let reg = registry();

        println!("\n=== API Bindings Registry ===\n");

        for category in reg.categories() {
            let methods = reg.methods_by_category(category);
            println!("{}:", category);
            for method in methods {
                println!(
                    "  - {} (js: {}, py: {})",
                    method.internal_name, method.js_name, method.py_name
                );
            }
            println!();
        }
    }
}
