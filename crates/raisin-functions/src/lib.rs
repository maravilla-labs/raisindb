// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

// TODO(v0.2): Clean up unused code
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unexpected_cfgs)]

//! # Raisin Functions
//!
//! Serverless functions for RaisinDB enabling custom logic execution in
//! JavaScript (QuickJS) and Starlark (Python-like) runtimes.
//!
//! ## Features
//!
//! - **Multiple Runtimes**: JavaScript (QuickJS), Starlark (Python-like), SQL
//! - **Sandboxed Execution**: Resource limits, timeouts, memory constraints
//! - **RaisinDB API Access**: Node operations, SQL queries, events
//! - **Allowlisted HTTP**: Controlled external API access
//! - **Flexible Triggers**: Event-driven, scheduled, HTTP, SQL calls
//!
//! ## Example JavaScript Function
//!
//! ```javascript
//! async function handler(input) {
//!     const node = await raisin.nodes.get("default", input.path);
//!     await raisin.nodes.update("default", input.path, {
//!         properties: { ...node.properties, status: "processed" }
//!     });
//!     return { success: true };
//! }
//! ```

pub mod api;
pub mod execution;
pub mod executor;
pub mod loader;
pub mod media_capabilities;
pub mod plugin;
pub mod plugin_loader;
pub mod runtime;
pub mod types;

// Re-exports
pub use api::{
    // Callback types for building RaisinFunctionApi
    EmitEventCallback,
    FunctionApi,
    HttpRequestCallback,
    NodeCreateCallback,
    NodeDeleteCallback,
    NodeGetByIdCallback,
    NodeGetCallback,
    NodeGetChildrenCallback,
    NodeQueryCallback,
    NodeUpdateCallback,
    RaisinFunctionApi,
    RaisinFunctionApiCallbacks,
    SqlExecuteCallback,
    SqlQueryCallback,
};
pub use execution::{
    configure_mcp_client, configure_platform_hooks, shared_http_client, PlatformHook,
};
pub use executor::{ExecutionHandle, FunctionExecutor};
pub use loader::FunctionLoader;
pub use media_capabilities::{
    capability_report, log_capability_report, CapabilityRow, Provider, MEDIA_KINDS,
};
pub use plugin::{
    plugin_manifest, register_function_plugin, registered_plugin_methods, rejected_plugins,
    FunctionBindingPlugin, PluginManifestEntry, PluginRejection,
};
pub use plugin_loader::{load_plugins_from_dir, RAISIN_PLUGIN_ABI_VERSION};
pub use runtime::{FunctionRuntime, RuntimeRegistry};
// Installed process-wide by `raisin-server` from `[functions.wasm]`, the same
// inversion `configure_mcp_client` uses. Re-exported at the crate root so the
// call site does not have to know which module the engine lives in.
#[cfg(feature = "wasm")]
pub use runtime::wasm::{
    compile_count, configure_wasm_runtime, WasmAllocationStrategy, WasmRuntimeConfig,
    HOST_ABI_VERSION,
};
// Upload-time and install-time artifact validation, plus the boot-time engine
// build. Present in EVERY build: with the feature they are the real compile and
// the real engine, without it a warn-once accept and a no-op, so the HTTP upload
// path, the package installer and `main.rs` never cfg-branch. An async caller
// must use `validate_component_async` — a compile is seconds of CPU and does
// not belong on a tokio worker.
pub use runtime::{
    init_wasm_engine, max_wasm_artifact_bytes, validate_component, validate_component_async,
};
pub use types::*;
