// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Console API and timer setup for the QuickJS runtime.
//!
//! Provides the `console` object (log, debug, info, warn, error) with
//! log capture, and the timer internal API (setTimeout, setInterval).

use rquickjs::{prelude::Rest, CaughtError, Ctx, Function, Object, Promise, Value};
use std::sync::{Arc, Mutex};

use crate::runtime::timers::TimerRegistry;
use crate::types::LogEntry;

/// Convert a JavaScript Value to a string for console output.
///
/// - Strings are used directly (no quotes, like browser console)
/// - Numbers and booleans are converted with to_string()
/// - Objects and arrays are JSON stringified with pretty formatting (2-space indent)
/// - null/undefined become "null"/"undefined"
pub(super) fn value_to_console_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> String {
    if value.is_undefined() {
        return "undefined".to_string();
    }
    if value.is_null() {
        return "null".to_string();
    }
    if let Some(s) = value.as_string() {
        // Return string value directly without quotes (like browser console)
        return s.to_string().unwrap_or_else(|_| "[string]".to_string());
    }
    if let Some(n) = value.as_int() {
        return n.to_string();
    }
    if let Some(n) = value.as_float() {
        return n.to_string();
    }
    if let Some(b) = value.as_bool() {
        return b.to_string();
    }

    // For objects and arrays, use JSON.stringify then pretty-print with serde
    match ctx.json_stringify(value) {
        Ok(Some(js_str)) => {
            let json_str = js_str
                .to_string()
                .unwrap_or_else(|_| "[object]".to_string());
            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(parsed) => serde_json::to_string_pretty(&parsed).unwrap_or(json_str),
                Err(_) => json_str,
            }
        }
        Ok(None) => "undefined".to_string(),
        Err(_) => "[object]".to_string(),
    }
}

/// Create the console API object with log capture.
pub(super) fn create_console_api<'js>(
    ctx: &Ctx<'js>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    log_emitter: Option<raisin_storage::LogEmitter>,
) -> std::result::Result<Object<'js>, rquickjs::Error> {
    let console = Object::new(ctx.clone())?;

    // console.log
    let logs_log = logs.clone();
    let emitter_log = log_emitter.clone();
    let log_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let parts: Vec<String> = args
            .0
            .iter()
            .map(|v| value_to_console_string(&ctx, v.clone()))
            .collect();
        let message = parts.join(" ");
        tracing::info!(target: "js_console", "{}", message);

        if let Ok(mut guard) = logs_log.lock() {
            guard.push(LogEntry::info(&message));
        }
        if let Some(ref emitter) = emitter_log {
            emitter("info".to_string(), message);
        }

        Ok::<_, rquickjs::Error>(())
    })?;
    console.set("log", log_fn)?;

    // console.debug
    let logs_debug = logs.clone();
    let emitter_debug = log_emitter.clone();
    let debug_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let parts: Vec<String> = args
            .0
            .iter()
            .map(|v| value_to_console_string(&ctx, v.clone()))
            .collect();
        let message = parts.join(" ");
        tracing::debug!(target: "js_console", "{}", message);

        if let Ok(mut guard) = logs_debug.lock() {
            guard.push(LogEntry::debug(&message));
        }
        if let Some(ref emitter) = emitter_debug {
            emitter("debug".to_string(), message);
        }

        Ok::<_, rquickjs::Error>(())
    })?;
    console.set("debug", debug_fn)?;

    // console.info
    let logs_info = logs.clone();
    let emitter_info = log_emitter.clone();
    let info_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let parts: Vec<String> = args
            .0
            .iter()
            .map(|v| value_to_console_string(&ctx, v.clone()))
            .collect();
        let message = parts.join(" ");
        tracing::info!(target: "js_console", "{}", message);

        if let Ok(mut guard) = logs_info.lock() {
            guard.push(LogEntry::info(&message));
        }
        if let Some(ref emitter) = emitter_info {
            emitter("info".to_string(), message);
        }

        Ok::<_, rquickjs::Error>(())
    })?;
    console.set("info", info_fn)?;

    // console.warn
    let logs_warn = logs.clone();
    let emitter_warn = log_emitter.clone();
    let warn_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let parts: Vec<String> = args
            .0
            .iter()
            .map(|v| value_to_console_string(&ctx, v.clone()))
            .collect();
        let message = parts.join(" ");
        tracing::warn!(target: "js_console", "{}", message);

        if let Ok(mut guard) = logs_warn.lock() {
            guard.push(LogEntry::warn(&message));
        }
        if let Some(ref emitter) = emitter_warn {
            emitter("warn".to_string(), message);
        }

        Ok::<_, rquickjs::Error>(())
    })?;
    console.set("warn", warn_fn)?;

    // console.error
    let logs_error = logs.clone();
    let emitter_error = log_emitter.clone();
    let error_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let parts: Vec<String> = args
            .0
            .iter()
            .map(|v| value_to_console_string(&ctx, v.clone()))
            .collect();
        let message = parts.join(" ");
        tracing::error!(target: "js_console", "{}", message);

        if let Ok(mut guard) = logs_error.lock() {
            guard.push(LogEntry::error(&message));
        }
        if let Some(ref emitter) = emitter_error {
            emitter("error".to_string(), message);
        }

        Ok::<_, rquickjs::Error>(())
    })?;
    console.set("error", error_fn)?;

    Ok(console)
}

/// Format a JavaScript error for display.
pub(super) fn format_js_error<'js>(_ctx: &Ctx<'js>, error: CaughtError<'js>) -> String {
    match error {
        CaughtError::Error(e) => {
            let msg = format!("{}", e);
            if msg.contains("Exception generated by QuickJS") {
                "Runtime exception".to_string()
            } else {
                msg
            }
        }
        CaughtError::Exception(exc) => {
            let msg = exc.message().unwrap_or_else(|| "Unknown error".to_string());
            if let Some(stack) = exc.stack() {
                format!("{}\n{}", msg, stack)
            } else {
                msg
            }
        }
        CaughtError::Value(_val) => "Unknown error value".to_string(),
    }
}

/// Setup timer APIs (setTimeout, clearTimeout, setInterval, clearInterval).
///
/// This registers internal functions that the JavaScript polyfill uses:
/// - `__timers_internal.create_timer(delay_ms)` - Creates a timer and returns its ID
/// - `__timers_internal.wait_timer(timer_id, delay_ms)` - Returns a Promise that resolves after delay
/// - `__timers_internal.cancel_timer(timer_id)` - Cancels a pending timer
pub(super) fn setup_timers_api<'js>(
    ctx: &Ctx<'js>,
    timer_registry: Arc<TimerRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    let globals = ctx.globals();

    let internal = Object::new(ctx.clone())?;

    // create_timer - Generate a timer ID and register it
    let registry_clone = timer_registry.clone();
    let create_timer_fn = Function::new(ctx.clone(), move |_delay_ms: u32| {
        let timer_id = registry_clone.generate_id();
        Ok::<_, rquickjs::Error>(timer_id)
    })?;
    internal.set("create_timer", create_timer_fn)?;

    // wait_timer - Returns a Promise that resolves after the delay (or when cancelled)
    let registry_clone = timer_registry.clone();
    let wait_timer_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, timer_id: String, delay_ms: u32| {
            let registry = registry_clone.clone();

            let promise = Promise::wrap_future(&ctx, async move {
                let cancel_rx = registry.register(timer_id.clone());

                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)) => {
                        registry.remove(&timer_id);
                        Ok::<bool, rquickjs::Error>(true)
                    }
                    _ = cancel_rx => {
                        Ok(false)
                    }
                }
            })?;

            Ok::<_, rquickjs::Error>(promise)
        },
    )?;
    internal.set("wait_timer", wait_timer_fn)?;

    // cancel_timer - Cancel a pending timer
    let registry_clone = timer_registry.clone();
    let cancel_timer_fn = Function::new(ctx.clone(), move |timer_id: String| {
        registry_clone.cancel(&timer_id);
        Ok::<_, rquickjs::Error>(())
    })?;
    internal.set("cancel_timer", cancel_timer_fn)?;

    globals.set("__timers_internal", internal)?;

    Ok(())
}
