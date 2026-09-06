// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `[functions]` configuration — currently only the WebAssembly runtime.
//!
//! Mirrors `raisin_functions::WasmRuntimeConfig` field for field. It is
//! mirrored rather than reused because `raisin-functions` is an optional
//! dependency (`storage-rocksdb`) while this config type is always compiled —
//! the same reasoning as [`super::PlatformHookConfig`] and
//! [`super::TriggerSafetyConfig`]. `main.rs` converts one into the other under
//! `#[cfg(feature = "wasm")]`, so the two shapes cannot drift silently: a field
//! added on either side fails that conversion to compile.

use serde::{Deserialize, Serialize};

/// `[functions]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionsConfig {
    /// `[functions.wasm]` — the WebAssembly component runtime.
    #[serde(default)]
    pub wasm: WasmFunctionsConfig,
}

/// How wasmtime backs guest linear memories (`allocation = "..."`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WasmAllocation {
    /// Allocate a store's memory when it is instantiated. The default: a
    /// sizing mistake costs a slow instantiation, not a server that will not
    /// boot.
    OnDemand,
    /// Reserve a pool of slots when the engine is built. Faster instantiation,
    /// but the reservation happens once and a too-large one fails at boot.
    Pooling,
}

/// `[functions.wasm]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmFunctionsConfig {
    /// Whether wasm functions may execute at all. `false` still registers the
    /// runtime, so the failure names this setting rather than the language.
    #[serde(default = "default_wasm_enabled")]
    pub enabled: bool,
    /// Largest artifact accepted at load, run-file and upload. 32 MiB by
    /// default: jco/StarlingMonkey components are 8-15 MB.
    #[serde(default = "default_max_artifact_bytes")]
    pub max_artifact_bytes: usize,
    /// Process-wide budget for compiled code, evicted LRU by weight.
    #[serde(default = "default_compiled_cache_bytes")]
    pub compiled_cache_bytes: u64,
    /// Engine-global guest stack ceiling. A `raisin:Function`'s
    /// `resource_limits.max_stack_bytes` cannot reach this — wasmtime fixes the
    /// stack size when the engine is built.
    #[serde(default = "default_max_wasm_stack_bytes")]
    pub max_wasm_stack_bytes: u64,
    /// Epoch ticker period. Timeout resolution is one tick.
    #[serde(default = "default_epoch_tick_ms")]
    pub epoch_tick_ms: u64,
    /// Memory allocation strategy.
    #[serde(default = "default_allocation")]
    pub allocation: WasmAllocation,
    /// Pooling sizing only: concurrent component instances the pool reserves.
    #[serde(default = "default_max_instances")]
    pub max_instances: u32,
    /// Per stream, per execution capture ceiling for guest stdout/stderr.
    #[serde(default = "default_stdout_capture_bytes")]
    pub stdout_capture_bytes: usize,
}

fn default_wasm_enabled() -> bool {
    true
}
fn default_max_artifact_bytes() -> usize {
    33_554_432
}
fn default_compiled_cache_bytes() -> u64 {
    268_435_456
}
fn default_max_wasm_stack_bytes() -> u64 {
    1_048_576
}
fn default_epoch_tick_ms() -> u64 {
    10
}
fn default_allocation() -> WasmAllocation {
    WasmAllocation::OnDemand
}
fn default_max_instances() -> u32 {
    15
}
fn default_stdout_capture_bytes() -> usize {
    1_048_576
}

impl Default for WasmFunctionsConfig {
    fn default() -> Self {
        Self {
            enabled: default_wasm_enabled(),
            max_artifact_bytes: default_max_artifact_bytes(),
            compiled_cache_bytes: default_compiled_cache_bytes(),
            max_wasm_stack_bytes: default_max_wasm_stack_bytes(),
            epoch_tick_ms: default_epoch_tick_ms(),
            allocation: default_allocation(),
            max_instances: default_max_instances(),
            stdout_capture_bytes: default_stdout_capture_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ServerConfigFile;
    use super::*;

    /// An absent `[functions]` section must produce the shipped defaults, not
    /// a disabled runtime.
    #[test]
    fn omitted_functions_section_defaults() {
        let config: ServerConfigFile =
            toml::from_str("[server]\nport = 8080\n").expect("minimal config must parse");
        let wasm = &config.functions.wasm;
        assert!(wasm.enabled);
        assert_eq!(wasm.max_artifact_bytes, 33_554_432);
        assert_eq!(wasm.compiled_cache_bytes, 268_435_456);
        assert_eq!(wasm.max_wasm_stack_bytes, 1_048_576);
        assert_eq!(wasm.epoch_tick_ms, 10);
        assert_eq!(wasm.allocation, WasmAllocation::OnDemand);
        assert_eq!(wasm.max_instances, 15);
        assert_eq!(wasm.stdout_capture_bytes, 1_048_576);
    }

    /// Every documented key lands where the code reads it, and a partial
    /// section keeps defaults for the keys it did not name.
    #[test]
    fn wasm_section_parses() {
        let config: ServerConfigFile = toml::from_str(
            "[functions.wasm]\nenabled = false\nmax_artifact_bytes = 1024\ncompiled_cache_bytes = 2048\nmax_wasm_stack_bytes = 4096\nepoch_tick_ms = 25\nallocation = \"pooling\"\nmax_instances = 3\nstdout_capture_bytes = 512\n",
        )
        .expect("[functions.wasm] must parse");
        let wasm = &config.functions.wasm;
        assert!(!wasm.enabled);
        assert_eq!(wasm.max_artifact_bytes, 1024);
        assert_eq!(wasm.compiled_cache_bytes, 2048);
        assert_eq!(wasm.max_wasm_stack_bytes, 4096);
        assert_eq!(wasm.epoch_tick_ms, 25);
        assert_eq!(wasm.allocation, WasmAllocation::Pooling);
        assert_eq!(wasm.max_instances, 3);
        assert_eq!(wasm.stdout_capture_bytes, 512);

        let partial: ServerConfigFile =
            toml::from_str("[functions.wasm]\nmax_artifact_bytes = 64\n").unwrap();
        assert_eq!(partial.functions.wasm.max_artifact_bytes, 64);
        assert!(partial.functions.wasm.enabled);
        assert_eq!(partial.functions.wasm.epoch_tick_ms, 10);
    }

    /// `allocation` is a closed set: a typo must fail at boot rather than
    /// silently selecting a strategy the operator did not ask for.
    #[test]
    fn unknown_allocation_strategy_is_rejected() {
        let err = toml::from_str::<ServerConfigFile>("[functions.wasm]\nallocation = \"pool\"\n")
            .expect_err("an unknown allocation strategy must be rejected");
        assert!(err.to_string().contains("pool"), "{err}");
    }
}
