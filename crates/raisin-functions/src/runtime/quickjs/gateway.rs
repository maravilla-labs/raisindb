// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Registry gateway for the QuickJS runtime.
//!
//! Registers a single host function, `__raisin_call(method, argsJson)`, that
//! dispatches to the shared bindings registry (`runtime/bindings/methods/*`).
//! This mirrors the Starlark gateway (`runtime/starlark/gateway.rs`): every
//! `raisin.*` method has exactly ONE Rust implementation — the registry
//! invoker — consumed by both runtimes.
//!
//! Contract with `api_wrapper.js`:
//! - `method` is the registry `internal_name` (e.g. `"nodes_get"`).
//! - `argsJson` is a JSON array of positional arguments
//!   (`JSON.stringify([...])` on the JS side; `undefined` becomes `null`,
//!   which optional-arg parsers treat as absent).
//! - The return value is always a JSON string:
//!   - success: the invoker result serialized via `InvokeResult::to_json_string()`
//!   - failure: `{"error":true,"message":"<Display of the error>"}`
//!
//! The gateway itself never throws for API failures — the JS wrapper applies
//! each method's documented error convention (throw / null / [] / sentinel /
//! returned error object) on top of this uniform envelope, so the public
//! surface behavior is preserved exactly.

use rquickjs::{Ctx, Function};
use std::sync::Arc;

use super::helpers::run_async_blocking;
use crate::api::FunctionApi;
use crate::runtime::bindings::methods::registry;

/// Serialize an error message into the gateway's error envelope.
fn error_envelope(message: impl std::fmt::Display) -> String {
    serde_json::to_string(&serde_json::json!({
        "error": true,
        "message": message.to_string(),
    }))
    .unwrap_or_else(|_| r#"{"error":true,"message":"Unknown error"}"#.to_string())
}

/// Register the `__raisin_call` global used by `api_wrapper.js`.
pub(super) fn register_registry_gateway<'js>(
    ctx: &Ctx<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    let call_fn = Function::new(
        ctx.clone(),
        move |method: String, args_json: String| -> String {
            let api = api.clone();

            let Some(descriptor) = registry().find_by_internal_name(&method) else {
                tracing::error!(method = %method, "__raisin_call: unknown method");
                return error_envelope(format!("Unknown raisin API method: {}", method));
            };

            let args: Vec<serde_json::Value> = match serde_json::from_str(&args_json) {
                Ok(serde_json::Value::Array(items)) => items,
                Ok(other) => vec![other],
                Err(e) => {
                    tracing::error!(method = %method, error = %e, "__raisin_call: invalid arguments JSON");
                    return error_envelope(format!("Invalid arguments for {}: {}", method, e));
                }
            };

            let invoker = descriptor.invoker;
            let result = run_async_blocking(async move { (invoker)(api, args).await });

            match result {
                Ok(invoke_result) => invoke_result.to_json_string(),
                Err(e) => {
                    tracing::error!(method = %method, error = %e, "raisin API call failed");
                    error_envelope(e)
                }
            }
        },
    )?;

    ctx.globals().set("__raisin_call", call_fn)?;
    Ok(())
}
