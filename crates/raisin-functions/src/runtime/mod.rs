// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function runtime implementations
//!
//! This module provides the runtime engines for executing user-defined functions
//! in different languages (JavaScript, Starlark, SQL).

#[cfg(all(test, feature = "wasm"))]
mod bench_runtimes;
pub mod bindings;
pub mod crypto;
pub mod email;
pub mod fetch;
pub mod imap;
mod quickjs;
mod sandbox;
mod sql;
mod starlark;
pub mod temp;
pub mod timers;
#[cfg(feature = "wasm")]
pub mod wasm;

pub use quickjs::QuickJsRuntime;
pub use sandbox::{Sandbox, SandboxConfig};
pub use sql::SqlRuntime;
pub use starlark::StarlarkRuntime;
#[cfg(feature = "wasm")]
pub use wasm::{
    compile_count, init_engine as init_wasm_engine, max_wasm_artifact_bytes, validate_component,
    validate_component_async, WasmRuntime,
};

/// The largest WebAssembly artifact this server accepts.
///
/// Every acceptance point — the loader, `POST /api/files/{repo}/run`, the
/// upload handler — asks this ONE function, so `[functions.wasm]
/// max_artifact_bytes` reaches all of them. Without the `wasm` feature there is
/// no configuration to read and the compiled-in default answers; nothing can
/// execute on such a build anyway, and a package carrying a `.wasm` must still
/// install.
#[cfg(not(feature = "wasm"))]
pub fn max_wasm_artifact_bytes() -> usize {
    crate::types::DEFAULT_MAX_WASM_ARTIFACT_BYTES
}

/// Decide whether a WebAssembly artifact could ever run on this host.
///
/// This is the build WITHOUT the `wasm` feature, where there is no engine to
/// ask — so it warns once and accepts. That is deliberate: a stock server must
/// still be able to install a package that happens to contain a `.wasm` file
/// it will never execute. Refusing here would make the artifact undeployable
/// on the very servers that do not care about it.
///
/// The real implementation is `runtime::wasm::validate_component`, and both are
/// reached through this one path name, so a caller never cfg-branches.
#[cfg(not(feature = "wasm"))]
pub fn validate_component(_bytes: &[u8]) -> Result<()> {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "A WebAssembly artifact was accepted without validation: this server \
             was built without the `wasm` feature, so it has no engine to check \
             it against and cannot run it either"
        );
    });
    Ok(())
}

/// [`validate_component`] for an async caller, without the `wasm` feature.
///
/// The real implementation hands Cranelift to `spawn_blocking`, because a
/// compile is seconds of CPU and must never run on a tokio worker. There is no
/// compile here, so this is the accepting stub again — the point of the name is
/// that the upload handler and the package installer call it unconditionally.
#[cfg(not(feature = "wasm"))]
pub async fn validate_component_async(bytes: std::sync::Arc<[u8]>) -> Result<()> {
    validate_component(&bytes)
}

/// Build the WebAssembly engine now, so an invalid `[functions.wasm]` is a boot
/// failure rather than a panic inside the first request that needs it.
///
/// Without the `wasm` feature there is no engine to build and nothing to check,
/// so this succeeds; `main.rs` calls it unconditionally.
#[cfg(not(feature = "wasm"))]
pub fn init_wasm_engine() -> Result<()> {
    Ok(())
}

use crate::api::FunctionApi;
use crate::types::{
    ExecutionContext, ExecutionResult, FunctionCode, FunctionLanguage, FunctionMetadata,
};
use async_trait::async_trait;
use raisin_error::Result;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

/// Shared semaphore limiting concurrent function executions across all runtimes.
/// Each execution uses `block_in_place()` which pins a tokio worker thread.
/// Configurable via `RAISIN_MAX_CONCURRENT_FUNCTIONS` env var (default: 15).
pub(crate) static FUNCTION_EXECUTION_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    let max_concurrent = std::env::var("RAISIN_MAX_CONCURRENT_FUNCTIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15);
    tracing::info!(
        max_concurrent,
        "Function execution concurrency limit initialized"
    );
    Semaphore::new(max_concurrent)
});

/// Trait for function runtime engines
///
/// Each runtime implements execution for a specific language (JavaScript, Starlark, SQL).
#[async_trait]
pub trait FunctionRuntime: Send + Sync {
    /// Execute a function with the given code and context
    ///
    /// # Arguments
    /// * `code` - The function payload (entry file content): source text for
    ///   JavaScript/Starlark/SQL, opaque bytes for a WebAssembly component.
    ///   Text runtimes start with `let code = code.as_text()?;`
    /// * `entrypoint` - The name of the function to call (e.g., "handler")
    /// * `context` - Execution context with input, tenant, etc.
    /// * `metadata` - Function metadata (resource limits, network policy)
    /// * `api` - API implementation for node/SQL/HTTP operations
    /// * `files` - All function files for module resolution (path -> content)
    ///
    /// # Returns
    /// Execution result with output, stats, and any errors
    async fn execute(
        &self,
        code: &FunctionCode,
        entrypoint: &str,
        context: ExecutionContext,
        metadata: &FunctionMetadata,
        api: Arc<dyn FunctionApi>,
        files: HashMap<String, String>,
    ) -> Result<ExecutionResult>;

    /// Validate function code syntax without executing
    ///
    /// Used when creating/updating functions to catch syntax errors early.
    fn validate(&self, code: &FunctionCode) -> Result<()>;

    /// Get the language this runtime supports
    fn language(&self) -> FunctionLanguage;

    /// Get runtime name for logging/debugging
    fn name(&self) -> &'static str;
}

/// Registry of available function runtimes
pub struct RuntimeRegistry {
    runtimes: HashMap<FunctionLanguage, Arc<dyn FunctionRuntime>>,
}

impl RuntimeRegistry {
    /// Create a new runtime registry with default runtimes
    pub fn new() -> Self {
        let mut runtimes: HashMap<FunctionLanguage, Arc<dyn FunctionRuntime>> = HashMap::new();

        // Register QuickJS for JavaScript
        runtimes.insert(
            FunctionLanguage::JavaScript,
            Arc::new(QuickJsRuntime::new()),
        );

        // Register Starlark for Python-like (placeholder)
        runtimes.insert(FunctionLanguage::Starlark, Arc::new(StarlarkRuntime::new()));

        // Register SQL passthrough
        runtimes.insert(FunctionLanguage::Sql, Arc::new(SqlRuntime::new()));

        // Register the WebAssembly component runtime. Feature-gated because it
        // links Cranelift: only raisin-server turns it on, so every other
        // crate's tests keep building without it.
        #[cfg(feature = "wasm")]
        runtimes.insert(FunctionLanguage::Wasm, Arc::new(WasmRuntime::new()));

        Self { runtimes }
    }

    /// Get runtime for a specific language
    pub fn get(&self, language: FunctionLanguage) -> Option<Arc<dyn FunctionRuntime>> {
        self.runtimes.get(&language).cloned()
    }

    /// Register a custom runtime
    pub fn register(&mut self, runtime: Arc<dyn FunctionRuntime>) {
        self.runtimes.insert(runtime.language(), runtime);
    }

    /// List available languages
    pub fn available_languages(&self) -> Vec<FunctionLanguage> {
        self.runtimes.keys().cloned().collect()
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
