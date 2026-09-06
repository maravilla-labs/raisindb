// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Process-wide configuration for the WebAssembly runtime.
//!
//! Installed once at boot by `raisin-server` (`[functions.wasm]`), the same
//! inversion `configure_mcp_client` and `install_capability_probe` use: this
//! crate sits below the config crate, so the value is pushed down rather than
//! read up. A second install is logged and ignored — the engine is built from
//! these numbers (at boot, by `init_wasm_engine`) and cannot be rebuilt
//! underneath live stores.

use std::sync::OnceLock;

use crate::types::DEFAULT_MAX_WASM_ARTIFACT_BYTES;

/// How wasmtime backs guest linear memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmAllocationStrategy {
    /// Allocate each store's memory when it is instantiated. The default:
    /// nothing is reserved up front, so a sizing mistake costs a slow
    /// instantiation rather than a server that will not boot.
    OnDemand,
    /// Reserve a pool of slots at engine construction. Faster instantiation,
    /// but the reservation happens once and a too-large one fails at boot —
    /// literally at boot, because `main.rs` calls `init_wasm_engine` straight
    /// after installing this configuration rather than leaving the engine to be
    /// built by the first invocation.
    Pooling,
}

/// Tunables for the WebAssembly runtime, mirroring `[functions.wasm]`.
#[derive(Debug, Clone)]
pub struct WasmRuntimeConfig {
    /// Whether wasm functions may execute at all. `false` still registers the
    /// runtime, so the failure names the setting instead of the language.
    pub enabled: bool,
    /// Largest artifact accepted at load, run-file and upload.
    pub max_artifact_bytes: usize,
    /// Process-wide budget for compiled code, evicted LRU by weight.
    pub compiled_cache_bytes: u64,
    /// Engine-global guest stack ceiling. `ResourceLimits.max_stack_bytes` is
    /// a per-function value and cannot reach this: wasmtime fixes the stack
    /// size when the engine is built.
    pub max_wasm_stack_bytes: usize,
    /// Epoch ticker period. The timeout resolution is one tick.
    pub epoch_tick_ms: u64,
    /// Memory allocation strategy.
    pub allocation: WasmAllocationStrategy,
    /// Pooling sizing only: concurrent component instances the pool reserves.
    pub max_instances: u32,
    /// Per stream, per execution capture ceiling for guest stdout/stderr.
    pub stdout_capture_bytes: usize,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_artifact_bytes: DEFAULT_MAX_WASM_ARTIFACT_BYTES,
            compiled_cache_bytes: 256 * 1024 * 1024,
            max_wasm_stack_bytes: 1024 * 1024,
            epoch_tick_ms: 10,
            allocation: WasmAllocationStrategy::OnDemand,
            max_instances: 15,
            stdout_capture_bytes: 1024 * 1024,
        }
    }
}

static CONFIG: OnceLock<WasmRuntimeConfig> = OnceLock::new();

/// Install the process-wide wasm runtime configuration.
///
/// Call once, early — next to `load_plugins_from_dir` in `main.rs`, before any
/// function can execute, and follow it with `init_wasm_engine` so a
/// configuration wasmtime refuses is a boot failure with a message rather than
/// an error inside the first request that reaches wasm. A later call cannot
/// take effect (the engine is built from the first one) so it is logged and
/// dropped rather than silently replacing values that live stores already
/// depend on.
pub fn configure_wasm_runtime(config: WasmRuntimeConfig) {
    if CONFIG.set(config).is_err() {
        tracing::warn!(
            "configure_wasm_runtime called twice; the wasm engine is already built \
             from the first configuration and the second is ignored"
        );
    }
}

/// Whether a configuration was explicitly installed (as opposed to defaulted).
pub fn installed() -> bool {
    CONFIG.get().is_some()
}

/// The active configuration, defaulted when none was installed.
pub fn config() -> &'static WasmRuntimeConfig {
    CONFIG.get_or_init(WasmRuntimeConfig::default)
}

/// The artifact ceiling every acceptance point measures against.
///
/// Load, run-file and upload all ask THIS, never
/// [`crate::types::DEFAULT_MAX_WASM_ARTIFACT_BYTES`] directly: the constant is
/// the default this configuration starts from, and a second reader of it would
/// be a cap the operator raised in `[functions.wasm]` and one path silently
/// kept at 32 MiB.
pub fn max_artifact_bytes() -> usize {
    config().max_artifact_bytes
}
