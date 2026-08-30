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

/// Optional `raisin.<ns>.*` namespaces a distribution MAY supply with a plugin
/// and that core deliberately does not implement.
///
/// This is the ONE list, and it exists so a build with NO plugin still installs
/// a *soft-failing* namespace instead of leaving `raisin.media` `undefined`.
/// Before it, an absent plugin was a `TypeError` thrown at the guest's first
/// property access — while every document in the estate described the absent
/// case as `{ ok:false, reason:... }` data. A TypeError aborts the whole
/// invocation, so a job poller never reached its own failure handling: the job
/// node stayed `pending` forever with nothing recorded anywhere.
///
/// Listing a namespace here costs nothing when the plugin IS present: the
/// plugin's own snippet is appended afterwards and replaces the stub.
///
/// ONLY TOP-LEVEL, WHOLLY-OWNED NAMESPACES BELONG HERE. The Maravilla stripe
/// plugin is deliberately absent: it installs the NESTED
/// `globalThis.raisin.maravilla.stripe` via the
/// `raisin.maravilla = raisin.maravilla || {}` idiom, so a Proxy stub on
/// `raisin.maravilla` would be truthy, survive the `||`, and then shadow the
/// plugin's own `.stripe` assignment behind the stub's `get` trap — turning a
/// WORKING plugin into a soft failure. A nested namespace needs the stub
/// installed at its own leaf, which is a change to the plugin's wrapper, not to
/// this list.
pub const OPTIONAL_PLUGIN_NAMESPACES: &[&str] = &["media"];

/// The reason code a stubbed (absent-plugin) namespace returns. Distinct from
/// the plugin's own `not_configured` (loaded, but no endpoint/credentials) and
/// from `unknown_method` (loaded and configured, method name is a typo).
pub const PLUGIN_ABSENT_REASON: &str = "plugin_absent";

/// A plugin file the loader refused, kept so an operator can be told WHY at
/// startup and over the management API instead of grepping the journal.
#[derive(Clone, Debug)]
pub struct PluginRejection {
    /// Path of the shared library that was skipped.
    pub path: String,
    /// Reason (ABI mismatch, missing symbol, bad methods JSON, dlopen error…).
    pub reason: String,
}

fn rejection_slot() -> &'static RwLock<Vec<PluginRejection>> {
    static REJECTED: OnceLock<RwLock<Vec<PluginRejection>>> = OnceLock::new();
    REJECTED.get_or_init(|| RwLock::new(Vec::new()))
}

/// Record a plugin file the loader refused. Called by `plugin_loader`.
pub fn record_plugin_rejection(path: impl Into<String>, reason: impl Into<String>) {
    rejection_slot()
        .write()
        .expect("plugin rejection registry poisoned")
        .push(PluginRejection {
            path: path.into(),
            reason: reason.into(),
        });
}

/// Every plugin file the loader refused this process, in load order.
pub fn rejected_plugins() -> Vec<PluginRejection> {
    rejection_slot()
        .read()
        .expect("plugin rejection registry poisoned")
        .clone()
}

/// One registered plugin, flattened for reporting (startup log, management API).
#[derive(Clone, Debug, serde::Serialize)]
pub struct PluginManifestEntry {
    /// The plugin's self-declared name.
    pub name: String,
    /// Fully-qualified method names (`"<ns>.<method>"`) it services.
    pub methods: Vec<String>,
}

/// A reporting view of the plugin registry: what is loaded and what it can do.
///
/// This is the single source that the JS `raisin.capabilities` object, the
/// startup capability report and any management endpoint all read. The estate's
/// only capability probe before this was hand-written inside one Studio function
/// (`typeof raisin.media.screenshot === 'function'`) — exactly the mirrored-path
/// drift CLAUDE.md names as this repo's #1 recurring bug class, sitting on the
/// plugin boundary.
pub fn plugin_manifest() -> Vec<PluginManifestEntry> {
    registered_function_plugins()
        .iter()
        .map(|p| {
            let mut methods: Vec<String> = p.handlers().into_keys().collect();
            methods.sort();
            PluginManifestEntry {
                name: p.name().to_string(),
                methods,
            }
        })
        .collect()
}

/// Every method name serviced by some registered plugin, sorted and deduped.
pub fn registered_plugin_methods() -> Vec<String> {
    let mut methods: Vec<String> = all_plugin_handlers().into_keys().collect();
    methods.sort();
    methods.dedup();
    methods
}

/// True when some registered plugin services `method` (e.g. `"media.doc.toPdf"`).
pub fn plugin_method_available(method: &str) -> bool {
    all_plugin_handlers().contains_key(method)
}

/// True when some registered plugin services at least one method in `ns`.
pub fn plugin_namespace_available(ns: &str) -> bool {
    let prefix = format!("{ns}.");
    all_plugin_handlers()
        .keys()
        .any(|m| m.starts_with(&prefix) || m == ns)
}

/// The JS appended after `api_wrapper.js`.
///
/// ALWAYS non-empty, even with no plugins registered, because two things must
/// exist in every build:
///
/// 1. `raisin.capabilities` — the probe. `has('media.doc.toPdf')` answers
///    "can this server actually do it?" without duck-typing a namespace.
/// 2. A soft-failing stub for every [`OPTIONAL_PLUGIN_NAMESPACES`] entry no
///    plugin provides, so calling an absent binding returns
///    `{ ok:false, reason:"plugin_absent", method }` instead of throwing.
///
/// Plugin snippets are appended last, so a loaded plugin's real namespace
/// replaces its stub.
pub fn assemble_plugin_js() -> String {
    let plugins = registered_function_plugins();
    let methods = registered_plugin_methods();
    let names: Vec<String> = plugins.iter().map(|p| p.name().to_string()).collect();

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

    // ---- The probe ------------------------------------------------------
    // Serialised as JSON so a plugin name or method containing a quote cannot
    // break out of the generated source.
    let methods_json = serde_json::to_string(&methods).unwrap_or_else(|_| "[]".to_string());
    let names_json = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
    src.push_str(&format!(
        "const __capMethods = {methods_json};\n\
         const __capPlugins = {names_json};\n\
         globalThis.raisin.capabilities = Object.freeze({{\n\
         \x20 plugins: Object.freeze(__capPlugins.slice()),\n\
         \x20 methods: Object.freeze(__capMethods.slice()),\n\
         \x20 has: (m) => __capMethods.indexOf(m) !== -1,\n\
         \x20 namespace: (ns) => __capMethods.some((m) => m === ns || m.indexOf(ns + '.') === 0),\n\
         }});\n"
    ));

    // ---- Soft-failing stubs for absent optional namespaces --------------
    // A Proxy over a function target so BOTH `raisin.media.screenshot(...)` and
    // the nested `raisin.media.doc.toPdf(...)` work, and so the estate's
    // existing `typeof raisin.media.screenshot === 'function'` guard still
    // reports true — it then calls and receives the soft failure, rather than
    // silently taking a different branch on a server that HAS the plugin.
    src.push_str(&format!(
        "const __absentNs = (path) => new Proxy(function(){{}}, {{\n\
         \x20 get(_t, prop) {{\n\
         \x20   if (typeof prop === 'symbol') return undefined;\n\
         \x20   if (prop === 'then') return undefined;\n\
         \x20   if (prop === '__pluginAbsent') return true;\n\
         \x20   return __absentNs(path + '.' + String(prop));\n\
         \x20 }},\n\
         \x20 apply() {{ return {{ ok: false, reason: '{PLUGIN_ABSENT_REASON}', method: path }}; }},\n\
         }});\n"
    ));
    for ns in OPTIONAL_PLUGIN_NAMESPACES {
        if plugin_namespace_available(ns) {
            continue;
        }
        src.push_str(&format!(
            "if (globalThis.raisin['{ns}'] === undefined) {{ globalThis.raisin['{ns}'] = __absentNs('{ns}'); }}\n"
        ));
    }

    for plugin in &plugins {
        src.push_str(&plugin.js_wrapper());
        src.push('\n');
    }
    src.push_str("})();\n");
    src
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe and the stubs must exist in a build with NO plugins — that is
    /// the whole point. A public build used to get an empty string here, which
    /// is what made `raisin.media` `undefined` and turned an absent plugin into
    /// a thrown TypeError.
    #[test]
    fn probe_and_stubs_exist_without_any_plugin() {
        let js = assemble_plugin_js();
        assert!(js.contains("globalThis.raisin.capabilities"));
        assert!(js.contains(PLUGIN_ABSENT_REASON));
        for ns in OPTIONAL_PLUGIN_NAMESPACES {
            // Only namespaces no registered plugin provides get a stub; in a
            // unit-test process none are registered, so every one is stubbed.
            if !plugin_namespace_available(ns) {
                assert!(
                    js.contains(&format!("globalThis.raisin['{ns}']")),
                    "no stub emitted for optional namespace {ns}"
                );
            }
        }
    }

    #[test]
    fn manifest_and_method_lookup_agree() {
        // Whatever this process has registered, the flattened method list and
        // the per-method lookup must not disagree — they are the two shapes the
        // report and the JS probe read.
        for m in registered_plugin_methods() {
            assert!(plugin_method_available(&m));
            let ns = m.split('.').next().unwrap_or_default();
            assert!(plugin_namespace_available(ns));
        }
        let manifest_total: usize = plugin_manifest().iter().map(|e| e.methods.len()).sum();
        assert!(manifest_total >= registered_plugin_methods().len());
    }
}
