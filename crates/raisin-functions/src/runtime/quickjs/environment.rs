// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! JavaScript environment setup for the QuickJS runtime.
//!
//! Sets up the complete raisin API by registering internal Rust-backed
//! functions and evaluating JavaScript wrapper code that provides
//! the public API (raisin.nodes, raisin.sql, raisin.http, etc.).

use rquickjs::{Ctx, Object, Value};
use std::sync::{Arc, Mutex};

use crate::api::FunctionApi;
use crate::runtime::fetch::{AbortRegistry, StreamRegistry, FETCH_POLYFILL};
use crate::runtime::timers::{TimerRegistry, TIMERS_POLYFILL};
use crate::types::LogEntry;

use super::api_admin::register_admin_internal;
use super::api_fetch::register_fetch_internal;
use super::api_misc::{
    register_ai_internal, register_crypto_internal, register_events_internal,
    register_functions_internal, register_http_internal, register_sql_internal,
    register_tasks_internal,
};
use super::api_nodes::register_nodes_internal;
use super::api_resources::register_resources_internal;
use super::api_transaction::register_transaction_internal;
use super::console::{create_console_api, setup_timers_api};

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

    // Setup the raisin API (nodes, sql, http, events)
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

/// Register internal API functions that return JSON strings
/// then evaluate JS wrapper code that creates the nice public API.
fn setup_raisin_api<'js>(
    ctx: &Ctx<'js>,
    api: Arc<dyn FunctionApi>,
    abort_registry: Arc<AbortRegistry>,
    stream_registry: Arc<StreamRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    let globals = ctx.globals();

    // Create internal namespace for raw functions
    let internal = Object::new(ctx.clone())?;

    // Register all internal functions that return JSON strings
    register_nodes_internal(ctx, &internal, api.clone())?;
    register_sql_internal(ctx, &internal, api.clone())?;
    register_http_internal(ctx, &internal, api.clone())?;
    register_events_internal(ctx, &internal, api.clone())?;
    register_ai_internal(ctx, &internal, api.clone())?;
    register_resources_internal(ctx, &internal, api.clone())?;
    register_functions_internal(ctx, &internal, api.clone())?;
    register_tasks_internal(ctx, &internal, api.clone())?;
    register_transaction_internal(ctx, &internal, api.clone())?;
    register_admin_internal(ctx, &internal, api.clone())?;
    register_crypto_internal(ctx, &internal)?;

    // Register W3C Fetch API internal functions
    register_fetch_internal(ctx, &internal, api.clone(), abort_registry, stream_registry)?;

    globals.set("__raisin_internal", internal)?;

    // Evaluate JS code that creates the public API with JSON parsing
    ctx.eval::<(), _>(API_WRAPPER_JS.as_bytes().to_vec())?;

    Ok(())
}

/// JavaScript source that creates the public `globalThis.raisin` API.
///
/// This wraps the internal `__raisin_internal.*` functions (which return
/// JSON strings) into a developer-friendly API with parsed return values,
/// Resource class, transaction support, and admin escalation.
const API_WRAPPER_JS: &str = include_str!("api_wrapper.js");
