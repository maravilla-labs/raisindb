// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dynamic loading of function-binding plugins from shared libraries.
//!
//! A distribution (e.g. Maravilla) can drop a per-platform `.so`/`.dylib`/`.dll`
//! into the server's plugin directory; at startup the stock public raisindb
//! binary loads each one and registers its bindings — no custom server build,
//! no recompiling core. `load_plugins_from_dir` is called once during startup.
//!
//! ## Why a C ABI (not a Rust `dyn Trait` dylib)
//!
//! Rust has no stable ABI and no single shared global across a dlopen boundary,
//! so passing `Arc<dyn FunctionBindingPlugin>` from the plugin into the host —
//! or having the plugin call the host's `register_function_plugin` — is
//! fragile/broken (two copies of the registry). Instead the plugin exposes a
//! tiny **C ABI** (JSON over C strings); the loader wraps it in a HOST-side
//! [`CDylibPlugin`] that registers through the host's own registry. The plugin
//! therefore depends on NOTHING from raisindb and needs no toolchain pinning —
//! only these `#[no_mangle] extern "C"` symbols:
//!
//! ```c
//! uint32_t     raisin_plugin_abi_version(void);          // must equal RAISIN_PLUGIN_ABI_VERSION
//! const char*  raisin_plugin_name(void);                 // static, NUL-terminated
//! const char*  raisin_plugin_js_wrapper(void);           // static: installs raisin.<ns>.* via __pcall
//! const char*  raisin_plugin_methods(void);              // static: JSON array of "<ns>.<method>" names
//! char*        raisin_plugin_call(const char* method,    // heap JSON result; host frees via _free
//!                                 const char* args_json,  //   positional args array
//!                                 const char* ctx_json);  //   { tenant_id, repo_id, branch, workspace_id }
//! void         raisin_plugin_free(char* ptr);
//! ```

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Arc;

use libloading::{Library, Symbol};
use serde_json::{json, Value};

use crate::api::{PluginCallContext, PluginCallback};
use crate::plugin::{register_function_plugin, FunctionBindingPlugin};

/// Bump when the C ABI below changes incompatibly; the loader refuses a plugin
/// whose `raisin_plugin_abi_version()` differs.
pub const RAISIN_PLUGIN_ABI_VERSION: u32 = 1;

type AbiVersionFn = unsafe extern "C" fn() -> u32;
type StrFn = unsafe extern "C" fn() -> *const c_char;
type CallFn = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut c_char;
type FreeFn = unsafe extern "C" fn(*mut c_char);

/// Host-side adapter over a loaded plugin `.so`/`.dylib`/`.dll`. Holds the
/// `Library` alive for the process lifetime and forwards each method through the
/// plugin's C `call` symbol.
struct CDylibPlugin {
    _lib: Arc<Library>,
    name: String,
    js: String,
    methods: Vec<String>,
    call: CallFn,
    free: FreeFn,
}

impl FunctionBindingPlugin for CDylibPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn js_wrapper(&self) -> String {
        self.js.clone()
    }

    fn handlers(&self) -> HashMap<String, PluginCallback> {
        let mut map = HashMap::new();
        for method in &self.methods {
            let method_key = method.clone();
            let call = self.call;
            let free = self.free;
            let lib = self._lib.clone();
            let cb: PluginCallback = Arc::new(move |ctx: PluginCallContext, args: Value| {
                let method = method_key.clone();
                let lib = lib.clone();
                Box::pin(async move {
                    let ctx_json = serde_json::to_string(&json!({
                        "tenant_id": ctx.tenant_id,
                        "repo_id": ctx.repo_id,
                        "branch": ctx.branch,
                        "workspace_id": ctx.workspace_id,
                    }))
                    .unwrap_or_else(|_| "{}".to_string());
                    let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
                    // The plugin's C `call` is synchronous (blocking HTTP inside);
                    // run it off the async worker.
                    let joined = tokio::task::spawn_blocking(move || {
                        let _keep = lib; // keep the library loaded during the call
                        call_plugin(call, free, &method, &args_json, &ctx_json)
                    })
                    .await;
                    match joined {
                        Ok(Ok(v)) => Ok(v),
                        Ok(Err(reason)) => Ok(json!({ "ok": false, "reason": reason })),
                        Err(_) => Ok(json!({ "ok": false, "reason": "plugin_task_join_error" })),
                    }
                })
            });
            map.insert(method.clone(), cb);
        }
        map
    }
}

/// Marshal to C strings, invoke the plugin, copy + free the result, parse JSON.
fn call_plugin(
    call: CallFn,
    free: FreeFn,
    method: &str,
    args_json: &str,
    ctx_json: &str,
) -> Result<Value, String> {
    let m = CString::new(method).map_err(|_| "bad method string".to_string())?;
    let a = CString::new(args_json).map_err(|_| "bad args json".to_string())?;
    let c = CString::new(ctx_json).map_err(|_| "bad ctx json".to_string())?;
    // SAFETY: pointers are valid for the duration of the call; the returned
    // pointer is a heap C string owned by the plugin, freed via `free`.
    unsafe {
        let ptr = call(m.as_ptr(), a.as_ptr(), c.as_ptr());
        if ptr.is_null() {
            return Err("plugin_returned_null".to_string());
        }
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        free(ptr);
        serde_json::from_str(&s).map_err(|e| format!("plugin_bad_json: {e}"))
    }
}

unsafe fn read_static_str(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("plugin returned a null string".to_string());
    }
    Ok(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

fn is_shared_lib(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("so") | Some("dylib") | Some("dll")
    )
}

/// Load + register every shared-library plugin in `dir`. Missing dir → no-op.
/// Each plugin failure is logged and skipped (never fatal). Call ONCE at
/// startup, before any function executes.
pub fn load_plugins_from_dir(dir: impl AsRef<Path>) {
    let dir = dir.as_ref();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => {
            tracing::debug!(dir = %dir.display(), "no plugin directory; no function plugins loaded");
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_shared_lib(&path) {
            continue;
        }
        // SAFETY: loading arbitrary native code — the plugin dir is a trusted,
        // operator-controlled location (same trust level as the server binary).
        match unsafe { load_one(&path) } {
            Ok(name) => tracing::info!(plugin = %name, path = %path.display(), "loaded function plugin"),
            Err(e) => tracing::error!(path = %path.display(), error = %e, "skipping plugin"),
        }
    }
}

unsafe fn load_one(path: &Path) -> Result<String, String> {
    let lib = Library::new(path).map_err(|e| e.to_string())?;

    // Resolve symbols + copy out fn pointers / static strings, then drop the
    // borrowing `Symbol`s so the library can move into the Arc.
    let (name, js, methods, call, free) = {
        let abi: Symbol<AbiVersionFn> = lib
            .get(b"raisin_plugin_abi_version\0")
            .map_err(|e| format!("missing raisin_plugin_abi_version: {e}"))?;
        let v = abi();
        if v != RAISIN_PLUGIN_ABI_VERSION {
            return Err(format!(
                "ABI version mismatch (plugin={v}, host={RAISIN_PLUGIN_ABI_VERSION})"
            ));
        }
        let name_fn: Symbol<StrFn> = lib
            .get(b"raisin_plugin_name\0")
            .map_err(|e| format!("missing raisin_plugin_name: {e}"))?;
        let js_fn: Symbol<StrFn> = lib
            .get(b"raisin_plugin_js_wrapper\0")
            .map_err(|e| format!("missing raisin_plugin_js_wrapper: {e}"))?;
        let methods_fn: Symbol<StrFn> = lib
            .get(b"raisin_plugin_methods\0")
            .map_err(|e| format!("missing raisin_plugin_methods: {e}"))?;
        let call: Symbol<CallFn> = lib
            .get(b"raisin_plugin_call\0")
            .map_err(|e| format!("missing raisin_plugin_call: {e}"))?;
        let free: Symbol<FreeFn> = lib
            .get(b"raisin_plugin_free\0")
            .map_err(|e| format!("missing raisin_plugin_free: {e}"))?;

        let name = read_static_str(name_fn())?;
        let js = read_static_str(js_fn())?;
        let methods_json = read_static_str(methods_fn())?;
        let methods: Vec<String> = serde_json::from_str(&methods_json)
            .map_err(|e| format!("raisin_plugin_methods is not a JSON string array: {e}"))?;
        (name, js, methods, *call, *free)
    };

    let plugin = CDylibPlugin {
        _lib: Arc::new(lib),
        name: name.clone(),
        js,
        methods,
        call,
        free,
    };
    register_function_plugin(Arc::new(plugin));
    Ok(name)
}
