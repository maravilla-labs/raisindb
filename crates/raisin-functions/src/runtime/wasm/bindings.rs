// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Generated host bindings for `raisin:function` and the store data they run
//! against.
//!
//! The host surface is ONE generic gateway, exactly as in QuickJS
//! (`quickjs/gateway.rs`) and Starlark: every `raisin.*` method has a single
//! implementation — the registry invoker — and this runtime reaches it by
//! `internal_name`. Adding a host method must never mean editing this file.

use std::sync::Arc;

use wasmtime::component::{HasData, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::runtime::host_gateway::HostGateway;
use crate::types::{LogEntry, LogLevel};

wasmtime::component::bindgen!({
    world: "function",
    path: "wit",
    // Host imports are genuinely async: `InvokerFn` returns a `Send`
    // `BoxFuture`, so `call` awaits it directly instead of parking a worker
    // thread in `block_in_place` the way the two text runtimes must.
    imports: { default: async },
    exports: { default: async },
});

pub use self::raisin::function::host;

/// The semver this host implements. Bumped only when the WIT changes shape;
/// SDKs compare against it and refuse a host older than they were built for.
pub const HOST_ABI_VERSION: &str = "0.1.0";

/// Everything one execution's store owns.
///
/// A `Store<HostState>` is built fresh for every execution and dropped at the
/// end of it — see the isolation notes in `runtime_impl.rs`. Nothing here is
/// shared with another tenant.
pub struct HostState {
    /// The execution's host gateway: the API (already scoped to this tenant),
    /// the policy, the deadline and the stop. See `runtime/host_gateway.rs`.
    pub gateway: HostGateway,
    /// Logs collected from the `log` import; drained out of the store at the end.
    pub logs: Vec<LogEntry>,
    /// Real-time SSE emitter, when the caller is streaming.
    pub log_emitter: Option<raisin_storage::LogEmitter>,
    /// `FunctionApi::get_context()` rendered once, so a chatty guest cannot
    /// make N context calls into N clones of the whole context.
    pub context_json: String,
    /// Memory/table ceilings for this execution.
    pub limiter: super::limits::FunctionLimiter,
    /// WASI state: no args, no env, no preopens.
    pub wasi: WasiCtx,
    /// Resource handles the guest holds (streams, mainly).
    pub table: ResourceTable,
}

impl HostState {
    /// Record a log line both in the result and, when streaming, on the wire.
    fn record_log(&mut self, level: LogLevel, message: String) {
        tracing::info!(target: "wasm_console", "{}", message);
        if let Some(ref emitter) = self.log_emitter {
            emitter(level.to_string(), message.clone());
        }
        self.logs.push(LogEntry::new(level, message));
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// `HasData` marker letting the generated linker hand `&mut HostState` to the
/// host trait. Mirrors `wasmtime_wasi`'s own `WasiCli` / `WasiClocks` markers.
pub struct HasHostState;

impl HasData for HasHostState {
    type Data<'a> = &'a mut HostState;
}

/// The capability probe a wasm guest calls; JavaScript reads the same data as
/// `raisin.capabilities` (`crate::plugin::assemble_plugin_js`).
pub const CAPABILITIES_METHOD: &str = "capabilities.get";

impl HostState {
    async fn plugin_call(&mut self, method: String, args: String) -> Result<String, String> {
        let plugins: Vec<String> = crate::plugin::plugin_manifest()
            .into_iter()
            .map(|p| p.name)
            .collect();
        route_plugin_call(
            &self.gateway,
            &method,
            &args,
            &crate::plugin::registered_plugin_methods(),
            &plugins,
        )
        .await
    }
}

/// The JavaScript plugin surface, for a wasm guest. The JS wrapper hands
/// `__raisin_plugin_call(method, JSON.stringify(args))` the argument ARRAY, and
/// `FunctionApi::plugin_call` receives it unchanged; so does this. An optional
/// namespace with no loaded plugin answers the soft failure the JS stub returns
/// (`{ok:false, reason:"plugin_absent", method}`), so a guest's "is media here?"
/// branch behaves the same in both runtimes. `methods`/`plugins` are the
/// registry's view (`crate::plugin`), passed in so the routing is testable.
pub(crate) async fn route_plugin_call(
    gateway: &HostGateway,
    method: &str,
    args: &str,
    methods: &[String],
    plugins: &[String],
) -> Result<String, String> {
    if method == CAPABILITIES_METHOD {
        return Ok(serde_json::json!({ "plugins": plugins, "methods": methods }).to_string());
    }
    if !methods.iter().any(|m| m == method) {
        let ns = method.split('.').next().unwrap_or_default();
        let ns_loaded = methods
            .iter()
            .any(|m| m == ns || m.starts_with(&format!("{ns}.")));
        if crate::plugin::OPTIONAL_PLUGIN_NAMESPACES.contains(&ns) && !ns_loaded {
            return Ok(serde_json::json!({
                "ok": false, "reason": crate::plugin::PLUGIN_ABSENT_REASON, "method": method,
            })
            .to_string());
        }
        tracing::error!(method = %method, "wasm host call: unknown method");
        return Err(format!("Unknown raisin API method: {}", method));
    }
    let args: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(method = %method, error = %e, "wasm plugin call: invalid arguments JSON");
            return Err(format!("Invalid arguments for {}: {}", method, e));
        }
    };
    match gateway.plugin_call(method, args).await {
        Ok(value) => serde_json::to_string(&value)
            .map_err(|e| format!("plugin result not serializable: {}", e)),
        Err(e) => Err(e.message),
    }
}

#[cfg(test)]
mod plugin_route_tests {
    use super::*;
    use crate::api::MockFunctionApi;
    use crate::types::ExecutionPolicy;
    use serde_json::{json, Value};

    fn gateway(api: &Arc<MockFunctionApi>) -> HostGateway {
        HostGateway::new(api.clone(), ExecutionPolicy::Standard)
    }

    fn media() -> Vec<String> {
        vec![
            "media.browser.extractHtml".into(),
            "media.job".into(),
            "media.fetch".into(),
        ]
    }

    #[tokio::test]
    async fn a_plugin_method_reaches_plugin_call_with_the_argument_array() {
        let api = Arc::new(MockFunctionApi::new(json!({})));
        let out = route_plugin_call(
            &gateway(&api),
            "media.browser.extractHtml",
            r#"[{"path":"/x"}]"#,
            &media(),
            &["media".into()],
        )
        .await;
        let v: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(v["method"], "media.browser.extractHtml");
        assert_eq!(v["args"], json!([{ "path": "/x" }]));
        assert_eq!(
            api.plugin_calls(),
            vec![(
                "media.browser.extractHtml".to_string(),
                json!([{ "path": "/x" }])
            )]
        );
    }

    #[tokio::test]
    async fn an_absent_optional_namespace_soft_fails_like_the_js_stub() {
        let api = Arc::new(MockFunctionApi::new(json!({})));
        let out = route_plugin_call(&gateway(&api), "media.browser.extractHtml", "[]", &[], &[])
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v,
            json!({ "ok": false, "reason": "plugin_absent", "method": "media.browser.extractHtml" })
        );
        assert!(api.plugin_calls().is_empty());
    }

    #[tokio::test]
    async fn a_typo_in_a_loaded_namespace_and_a_foreign_name_are_errors() {
        let api = Arc::new(MockFunctionApi::new(json!({})));
        for m in ["media.browser.extractHTML", "nosuch.thing"] {
            let e = route_plugin_call(&gateway(&api), m, "[]", &media(), &["media".into()])
                .await
                .unwrap_err();
            assert_eq!(e, format!("Unknown raisin API method: {m}"));
        }
    }

    #[tokio::test]
    async fn the_capability_probe_lists_plugins_and_methods() {
        let api = Arc::new(MockFunctionApi::new(json!({})));
        let out = route_plugin_call(
            &gateway(&api),
            CAPABILITIES_METHOD,
            "[]",
            &media(),
            &["media".into()],
        )
        .await
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["plugins"], json!(["media"]));
        assert_eq!(v["methods"].as_array().unwrap().len(), 3);
    }
}

impl host::Host for HostState {
    /// The generic gateway. Mirrors `quickjs/gateway.rs` string for string:
    /// an unknown method, bad arguments and an API failure are all `Err`
    /// values the guest can see and handle. Only a host bug traps.
    async fn call(&mut self, method: String, args: String) -> Result<String, String> {
        // The policy answers first, for registry and plugin methods alike.
        self.gateway.check_allowed(&method).map_err(|e| e.message)?;
        if !HostGateway::is_registry_method(&method) {
            // Not a registry method: a plugin binding (`media.*`), the capability
            // probe, or unknown - exactly what a JavaScript function can reach.
            return self.plugin_call(method, args).await;
        }
        // Host imports are genuinely async: the gateway's call is awaited, so
        // its deadline and stop apply with no thread blocked.
        self.gateway
            .call_json(&method, &args)
            .await
            .map_err(|e| e.message)
    }

    async fn log(&mut self, level: host::LogLevel, message: String) {
        let level = match level {
            host::LogLevel::Debug => LogLevel::Debug,
            host::LogLevel::Info => LogLevel::Info,
            host::LogLevel::Warn => LogLevel::Warn,
            host::LogLevel::Error => LogLevel::Error,
        };
        self.record_log(level, message);
    }

    async fn context(&mut self) -> String {
        self.context_json.clone()
    }

    async fn abi_version(&mut self) -> String {
        HOST_ABI_VERSION.to_string()
    }
}
