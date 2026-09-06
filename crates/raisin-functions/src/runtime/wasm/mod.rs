// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! WebAssembly component runtime (`language: wasm`).
//!
//! A function here is a compiled **component**, not source: it has no readable
//! body on the server, and the node's `entry_file` names both the artifact and
//! the handler inside it (`main.wasm:on-order`). One artifact can serve many
//! `raisin:Function` nodes.
//!
//! The contract is `wit/raisin-function.wit`, the single source of truth the
//! guest SDKs copy. It is deliberately small: ONE generic host gateway
//! mirroring `__raisin_call`, so a new `raisin.*` method reaches wasm guests
//! by being added to the bindings registry and nowhere else.

mod bindings;
mod cache;
mod capture;
mod compile;
mod config;
mod engine;
mod errors;
mod limits;
mod runtime_impl;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_cache;
#[cfg(test)]
mod tests_validation;

pub use bindings::HOST_ABI_VERSION;
pub use cache::{compile_count, compile_count_of, validate_component, validate_component_async};
pub use compile::compile;
pub use config::{
    configure_wasm_runtime, installed as wasm_configured,
    max_artifact_bytes as max_wasm_artifact_bytes, WasmAllocationStrategy, WasmRuntimeConfig,
};
pub use engine::init as init_engine;
pub use runtime_impl::WasmRuntime;
