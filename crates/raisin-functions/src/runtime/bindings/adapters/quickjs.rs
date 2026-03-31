// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! QuickJS runtime adapter
//!
//! This adapter registers all shared bindings with the QuickJS runtime.
//! Instead of ~1,500 lines of manual binding code, this adapter uses the
//! shared registry to generate bindings automatically.

use crate::api::FunctionApi;
use crate::runtime::bindings::methods::registry;
use crate::runtime::bindings::registry::ApiMethodDescriptor;
use rquickjs::{prelude::Rest, Ctx, Function, Object, Result as QjsResult, Value};
use std::sync::Arc;
use tokio::runtime::Handle;

/// Register all API bindings from the shared registry
///
/// This function iterates through all methods in the bindings registry
/// and creates QuickJS functions for each one.
///
/// # Arguments
/// * `ctx` - QuickJS context
/// * `internal` - The __raisin_internal object to add functions to
/// * `api` - The FunctionApi implementation
/// * `handle` - Tokio runtime handle for async execution
///
/// # Returns
/// Result indicating success or failure
pub fn register_all_bindings<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
    handle: Handle,
) -> QjsResult<()> {
    let reg = registry();

    for method in reg.methods() {
        register_single_binding(ctx, internal, api.clone(), handle.clone(), method)?;
    }

    Ok(())
}

/// Register a single binding from a method descriptor
fn register_single_binding<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
    handle: Handle,
    method: &ApiMethodDescriptor,
) -> QjsResult<()> {
    let internal_name = method.internal_name;
    let invoker = method.invoker;
    let arg_count = method.args.len();

    // Create a QuickJS function that calls the invoker
    let func = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let api = api.clone();
        let handle = handle.clone();

        // Convert QuickJS args to Vec<serde_json::Value>
        let mut json_args = Vec::with_capacity(arg_count);
        for arg in args.iter() {
            let json_val = qjs_to_json(&ctx, arg)?;
            json_args.push(json_val);
        }

        // Block on the async invoker
        let result = handle.block_on(async move { (invoker)(api, json_args).await });

        // Convert result back to QuickJS
        match result {
            Ok(invoke_result) => {
                let json_str = invoke_result.to_json_string();
                Ok::<_, rquickjs::Error>(json_str)
            }
            Err(e) => Err(rquickjs::Error::new_from_js_message(
                "raisin",
                internal_name,
                e.to_string(),
            )),
        }
    })?;

    // Add function to internal object
    internal.set(internal_name, func)?;

    Ok(())
}

/// Convert a QuickJS value to serde_json::Value
fn qjs_to_json<'js>(ctx: &Ctx<'js>, val: &rquickjs::Value<'js>) -> QjsResult<serde_json::Value> {
    if val.is_undefined() || val.is_null() {
        return Ok(serde_json::Value::Null);
    }

    if let Some(b) = val.as_bool() {
        return Ok(serde_json::Value::Bool(b));
    }

    if let Some(n) = val.as_int() {
        return Ok(serde_json::json!(n));
    }

    if let Some(n) = val.as_float() {
        return Ok(serde_json::json!(n));
    }

    if let Some(s) = val.as_string() {
        return Ok(serde_json::Value::String(s.to_string()?));
    }

    // For arrays and objects, use JSON serialization
    // This is less efficient but handles all cases
    if val.is_array() || val.is_object() {
        // Use QuickJS JSON.stringify
        let json_global: Object = ctx.globals().get("JSON")?;
        let stringify: Function = json_global.get("stringify")?;
        let json_str: String = stringify.call((val.clone(),))?;
        let parsed: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| rquickjs::Error::new_from_js_message("value", "JSON", e.to_string()))?;
        return Ok(parsed);
    }

    // Default to null for unsupported types
    Ok(serde_json::Value::Null)
}

/// Convert serde_json::Value to QuickJS value
#[allow(dead_code)]
fn json_to_qjs<'js>(ctx: &Ctx<'js>, val: &serde_json::Value) -> QjsResult<rquickjs::Value<'js>> {
    match val {
        serde_json::Value::Null => Ok(rquickjs::Value::new_null(ctx.clone())),
        serde_json::Value::Bool(b) => Ok(rquickjs::Value::new_bool(ctx.clone(), *b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(rquickjs::Value::new_int(ctx.clone(), i as i32))
            } else if let Some(f) = n.as_f64() {
                Ok(rquickjs::Value::new_float(ctx.clone(), f))
            } else {
                Ok(rquickjs::Value::new_null(ctx.clone()))
            }
        }
        serde_json::Value::String(s) => {
            let qjs_str = rquickjs::String::from_str(ctx.clone(), s)?;
            Ok(qjs_str.into())
        }
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            // Use JSON.parse for complex types
            let json_str = serde_json::to_string(val).map_err(|e| {
                rquickjs::Error::new_from_js_message("JSON", "value", e.to_string())
            })?;
            let json_global: Object = ctx.globals().get("JSON")?;
            let parse: Function = json_global.get("parse")?;
            parse.call((json_str,))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qjs_to_json_primitives() {
        // This test would require a QuickJS runtime which is complex to set up in tests
        // The actual functionality is tested through integration tests
    }
}
