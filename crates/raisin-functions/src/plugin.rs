// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function-binding plugins — a generic extension point for the server-side
//! JS function runtime.
//!
//! A `FunctionBindingPlugin` lets an out-of-tree crate add new `raisin.<ns>.*`
//! bindings to the public runtime WITHOUT editing core: it contributes (a) a JS
//! snippet defining the namespace object on `globalThis.raisin`, and (b) the
//! trusted Rust callbacks those methods dispatch to. This mirrors the existing
//! `raisin-indexer` `IndexPlugin` pattern (compile-time trait registration into
//! a process-global registry).
//!
//! WHY THIS EXISTS (security + release cadence): tenant functions must never be
//! able to make loopback/internal HTTP calls (SSRF). A plugin binding runs in
//! trusted core, so it is the sanctioned way to expose an internal service (e.g.
//! a media/screenshot service) to tenant functions — the callback holds the
//! endpoint/credentials, the guest only sees a logical method. And because a
//! host binary registers plugins at startup, a downstream distribution can
//! ship its own bindings on its own release cadence without forking core.
//!
//! DISPATCH (deliberately bypasses the bindings registry): plugin methods do NOT
//! add `ApiMethodDescriptor`s. Instead the plugin's JS calls the native
//! `__raisin_plugin_call('<ns>.<method>', argsJson)` gateway (see
//! `runtime/quickjs/gateway.rs`), which routes to `FunctionApi::plugin_call`,
//! which looks the method up in `RaisinFunctionApiCallbacks::plugin_callbacks`.
//! Keeping plugin methods out of the registry means the registry↔wrapper parity
//! tests and every existing binding are untouched.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::api::PluginCallback;

/// A set of related `raisin.<ns>.*` bindings contributed to the runtime.
pub trait FunctionBindingPlugin: Send + Sync {
    /// Stable identifier for this plugin (diagnostics only).
    fn name(&self) -> &str;

    /// JavaScript that installs the plugin's namespace on `globalThis.raisin`.
    ///
    /// Evaluated once per isolate, appended after the core `api_wrapper.js` (same
    /// script scope), with a `__pcall(method, argsArray)` helper in scope that
    /// routes to this plugin's callbacks and throws on error. Example:
    ///
    /// ```js
    /// globalThis.raisin.media = {
    ///   screenshot: (request) => __pcall('media.screenshot', [request]),
    /// };
    /// ```
    fn js_wrapper(&self) -> String;

    /// The trusted callbacks servicing this plugin's methods, keyed by the
    /// logical method name (`"<ns>.<method>"`, matching the `__pcall` keys in
    /// [`js_wrapper`](Self::js_wrapper)). Each receives the guest's positional
    /// args as a JSON array `Value` and returns the method's JSON result.
    fn handlers(&self) -> HashMap<String, PluginCallback>;
}

fn slot() -> &'static RwLock<Vec<Arc<dyn FunctionBindingPlugin>>> {
    static PLUGINS: OnceLock<RwLock<Vec<Arc<dyn FunctionBindingPlugin>>>> = OnceLock::new();
    PLUGINS.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a function-binding plugin. Call at process startup — BEFORE any
/// function runs — so its JS + handlers are present when isolates are built.
/// Registering the same plugin name twice simply stacks both (last-writer wins
/// per method in [`all_plugin_handlers`]).
pub fn register_function_plugin(plugin: Arc<dyn FunctionBindingPlugin>) {
    slot()
        .write()
        .expect("plugin registry poisoned")
        .push(plugin);
}

/// Snapshot of the currently registered plugins.
pub fn registered_function_plugins() -> Vec<Arc<dyn FunctionBindingPlugin>> {
    slot().read().expect("plugin registry poisoned").clone()
}

/// Merged method-name → callback map across all registered plugins. Wired into
/// `RaisinFunctionApiCallbacks::plugin_callbacks` by the callback factory.
pub fn all_plugin_handlers() -> HashMap<String, PluginCallback> {
    let mut merged = HashMap::new();
    for plugin in registered_function_plugins() {
        for (method, cb) in plugin.handlers() {
            merged.insert(method, cb);
        }
    }
    merged
}

/// The JS to append after `api_wrapper.js` for the registered plugins, or an
/// empty string when none are registered (public builds → zero change).
pub fn assemble_plugin_js() -> String {
    let plugins = registered_function_plugins();
    if plugins.is_empty() {
        return String::new();
    }
    let mut src = String::from("\n;(function(){\n");
    // Helper in scope for every plugin snippet: route to the native plugin
    // gateway, apply the standard error envelope (throw on { error:true }).
    src.push_str(
        "const __pcall = (method, args) => {\n\
         \x20 const r = JSON.parse(__raisin_plugin_call(method, JSON.stringify(args || [])));\n\
         \x20 if (r && r.error === true) { throw new Error(r.message || ('plugin call failed: ' + method)); }\n\
         \x20 return r;\n\
         };\n",
    );
    for plugin in &plugins {
        src.push_str(&plugin.js_wrapper());
        src.push('\n');
    }
    src.push_str("})();\n");
    src
}
