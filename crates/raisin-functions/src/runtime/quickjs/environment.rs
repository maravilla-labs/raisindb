// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! JavaScript environment setup for the QuickJS runtime.
//!
//! Sets up the complete raisin API:
//! - `__raisin_call` — the registry gateway (see `gateway.rs`); every
//!   `raisin.*` method dispatches through the shared bindings registry, the
//!   same single-definition-per-method registry the Starlark runtime uses.
//! - `__raisin_internal` — the few genuinely runtime-local host functions
//!   (per-execution temp files, W3C fetch plumbing).
//! - `api_wrapper.js` — the public `globalThis.raisin` surface (per-method
//!   error conventions, `Resource` class, transactions, admin escalation).

use rquickjs::{Ctx, Object, Value};
use std::sync::{Arc, Mutex};

use crate::api::FunctionApi;
use crate::runtime::fetch::{AbortRegistry, StreamRegistry, FETCH_POLYFILL};
use crate::runtime::timers::{TimerRegistry, TIMERS_POLYFILL};
use crate::types::LogEntry;

use super::api_fetch::register_fetch_internal;
use super::api_temp::register_temp_internal;
use super::console::{create_console_api, setup_timers_api};
use super::gateway::{register_plugin_gateway, register_registry_gateway};

/// Setup the JavaScript environment with the raisin API.
///
/// This provides the full raisin API including:
/// - raisin.context (execution context)
/// - raisin.nodes (node operations)
/// - raisin.sql (SQL queries)
/// - raisin.http (HTTP requests)
/// - raisin.events (event emission)
/// - console (logging with capture)
/// - W3C Fetch API (fetch, Request, Response, Headers, etc.)
pub(super) fn setup_js_environment<'js>(
    ctx: &Ctx<'js>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    context_data: &serde_json::Value,
    api: Arc<dyn FunctionApi>,
    log_emitter: Option<raisin_storage::LogEmitter>,
) -> std::result::Result<(), rquickjs::Error> {
    let globals = ctx.globals();

    // Create shared registries for fetch API
    let abort_registry = Arc::new(AbortRegistry::new());
    let stream_registry = Arc::new(StreamRegistry::new());

    // Create timer registry for setTimeout/setInterval
    let timer_registry = Arc::new(TimerRegistry::new());

    // Setup the raisin API (registry gateway + runtime-local host fns + JS wrapper)
    setup_raisin_api(
        ctx,
        api.clone(),
        abort_registry.clone(),
        stream_registry.clone(),
    )?;

    // Setup timer APIs (setTimeout, clearTimeout, setInterval, clearInterval)
    setup_timers_api(ctx, timer_registry)?;

    // Add context as a read-only property on the raisin object
    let context_json = serde_json::to_string(context_data).unwrap_or("{}".to_string());
    let context_bytes: Vec<u8> = context_json.into_bytes();
    let context_val: Value = ctx.json_parse(context_bytes)?;

    // Get the raisin object that was created by setup_raisin_api
    let raisin: Object = globals.get("raisin")?;
    raisin.set("context", context_val)?;

    // Create console object with log capture and real-time streaming
    let console = create_console_api(ctx, logs, log_emitter)?;
    globals.set("console", console)?;

    // Setup W3C Fetch API by evaluating the JavaScript polyfill
    // This creates: fetch, Request, Response, Headers, ReadableStream,
    // AbortController, AbortSignal, FormData, DOMException
    ctx.eval::<(), _>(FETCH_POLYFILL.as_bytes().to_vec())?;

    // Setup timer APIs by evaluating JavaScript polyfill
    // This creates: setTimeout, clearTimeout, setInterval, clearInterval
    ctx.eval::<(), _>(TIMERS_POLYFILL.as_bytes().to_vec())?;

    Ok(())
}

/// Register the registry gateway plus the runtime-local internal functions,
/// then evaluate the JS wrapper code that creates the public API.
fn setup_raisin_api<'js>(
    ctx: &Ctx<'js>,
    api: Arc<dyn FunctionApi>,
    abort_registry: Arc<AbortRegistry>,
    stream_registry: Arc<StreamRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    let globals = ctx.globals();

    // The single dispatch point for all raisin.* API methods: looks the
    // method up in the shared bindings registry and runs its invoker.
    register_registry_gateway(ctx, api.clone())?;

    // Gateway for function-binding plugin methods (raisin.<ns>.*). Routes to
    // FunctionApi::plugin_call, bypassing the registry. A no-op at the JS level
    // when no plugins are registered (nothing calls it).
    register_plugin_gateway(ctx, api.clone())?;

    // Internal namespace for the runtime-local host functions that cannot be
    // expressed as registry invokers (they bind per-execution state, not
    // FunctionApi): temp files and the W3C fetch plumbing.
    let internal = Object::new(ctx.clone())?;
    register_temp_internal(ctx, &internal)?;
    register_fetch_internal(ctx, &internal, api, abort_registry, stream_registry)?;
    globals.set("__raisin_internal", internal)?;

    // Evaluate JS code that creates the public API with JSON parsing, then the
    // registered function-binding plugins' snippets (empty in public builds).
    let mut wrapper_src = String::from(API_WRAPPER_JS);
    wrapper_src.push_str(&crate::plugin::assemble_plugin_js());
    ctx.eval::<(), _>(wrapper_src.into_bytes())?;

    Ok(())
}

/// JavaScript source that creates the public `globalThis.raisin` API.
///
/// This wraps `__raisin_call` (registry dispatch, JSON-string results) into a
/// developer-friendly API with parsed return values, per-method error
/// conventions, the Resource class, transaction support, and admin escalation.
const API_WRAPPER_JS: &str = include_str!("api_wrapper.js");
