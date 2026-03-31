// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Starlark runtime adapter
//!
//! This adapter registers all shared bindings with the Starlark runtime.
//! It creates Starlark native functions that call the shared invokers.

// Note: This module is a placeholder until the starlark crate is enabled.
// When enabled, it will implement binding registration similar to QuickJS.

/*
// Future implementation when starlark crate is enabled:

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::ApiMethodDescriptor;
use crate::runtime::bindings::methods::registry;
use starlark::environment::GlobalsBuilder;
use std::sync::Arc;
use tokio::runtime::Handle;

/// Register all API bindings from the shared registry for Starlark
///
/// # Arguments
/// * `builder` - Starlark GlobalsBuilder to add functions to
/// * `api` - The FunctionApi implementation
/// * `handle` - Tokio runtime handle for async execution
pub fn register_all_bindings(
    builder: &mut GlobalsBuilder,
    api: Arc<dyn FunctionApi>,
    handle: Handle,
) {
    let reg = registry();

    for method in reg.methods() {
        register_single_binding(builder, api.clone(), handle.clone(), method);
    }
}

fn register_single_binding(
    builder: &mut GlobalsBuilder,
    api: Arc<dyn FunctionApi>,
    handle: Handle,
    method: &ApiMethodDescriptor,
) {
    // Each binding is registered as an internal function
    // The Python wrapper code will expose these through the raisin.* namespace

    // Example for nodes_get:
    // builder.set(
    //     &format!("_internal_{}", method.internal_name),
    //     starlark::values::Value::new_function(move |args| {
    //         // Parse args, call invoker, return result
    //     })
    // );
}

/// Convert Starlark Value to serde_json::Value
fn starlark_to_json(val: &starlark::values::Value) -> serde_json::Value {
    // Implementation depends on Starlark API
    serde_json::Value::Null
}

/// Convert serde_json::Value to Starlark Value
fn json_to_starlark(heap: &starlark::values::Heap, val: &serde_json::Value) -> starlark::values::Value {
    // Implementation depends on Starlark API
    starlark::values::Value::new_none()
}
*/

// Placeholder implementation until starlark crate is enabled
pub fn register_all_bindings() {
    // No-op until starlark is enabled
}
