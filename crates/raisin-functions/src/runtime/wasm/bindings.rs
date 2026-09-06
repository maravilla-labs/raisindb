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

use crate::api::FunctionApi;
use crate::runtime::bindings::methods::registry;
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
    /// The API the gateway dispatches into, already scoped to this tenant.
    pub api: Arc<dyn FunctionApi>,
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

impl host::Host for HostState {
    /// The generic gateway. Mirrors `quickjs/gateway.rs` string for string:
    /// an unknown method, bad arguments and an API failure are all `Err`
    /// values the guest can see and handle. Only a host bug traps.
    async fn call(&mut self, method: String, args: String) -> Result<String, String> {
        let Some(descriptor) = registry().find_by_internal_name(&method) else {
            tracing::error!(method = %method, "wasm host call: unknown method");
            return Err(format!("Unknown raisin API method: {}", method));
        };

        let args: Vec<serde_json::Value> = match serde_json::from_str(&args) {
            Ok(serde_json::Value::Array(items)) => items,
            Ok(other) => vec![other],
            Err(e) => {
                tracing::error!(method = %method, error = %e, "wasm host call: invalid arguments JSON");
                return Err(format!("Invalid arguments for {}: {}", method, e));
            }
        };

        let invoker = descriptor.invoker;
        match (invoker)(self.api.clone(), args).await {
            Ok(result) => Ok(result.to_json_string()),
            Err(e) => {
                tracing::error!(method = %method, error = %e, "raisin API call failed");
                Err(e.to_string())
            }
        }
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
