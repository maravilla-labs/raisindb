// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Turning artifact bytes into something instantiable.
//!
//! ONE function does it, so the cache-miss path and the upload-time validator
//! cannot drift into disagreeing about what a valid artifact is. `cache.rs`
//! wraps it with a content-addressed cache and single-flight, and is the only
//! caller on the execution path; nothing else may compile.

use raisin_error::{Error, Result};
use wasmtime::component::{Component, InstancePre};

use super::bindings::{FunctionPre, HostState};
use super::engine::{engine, linker};

/// Compile and link one artifact.
///
/// Three checks, each with its own message, because "wasm component rejected"
/// with no reason is unactionable for whoever uploaded it:
///
/// 1. `Component::new` refuses a **core module** (or anything that is not a
///    component at all).
/// 2. `Linker::instantiate_pre` refuses an **unlinked import** — this is where
///    `wasi:sockets` and `wasi:http` are turned away, by name.
/// 3. `FunctionPre::new` refuses a component that does not **export
///    `handler`**, i.e. one built against a different world.
///
/// Cranelift is the expensive part (seconds on a 10 MB jco component), so
/// callers on an async path must run this inside `spawn_blocking`.
pub fn compile(bytes: &[u8]) -> Result<InstancePre<HostState>> {
    let component = Component::new(engine()?, bytes).map_err(|e| {
        Error::Validation(format!(
            "wasm component rejected: not a valid WebAssembly component \
             (a core module is not one) - {e:#}"
        ))
    })?;

    let instance_pre = linker()?.instantiate_pre(&component).map_err(|e| {
        Error::Validation(format!(
            "wasm component rejected: an import it needs is not provided by this host \
             (wasi:sockets and wasi:http are never provided) - {e:#}"
        ))
    })?;

    // Discarded: it only proves the export exists and has the right type. The
    // per-execution path rebuilds it from the cached `InstancePre`, which is
    // an index lookup, not a compile.
    FunctionPre::new(instance_pre.clone()).map_err(|e| {
        Error::Validation(format!(
            "wasm component rejected: it does not export \
             `handler: func(name: string, input: string) -> result<string, string>` - {e:#}"
        ))
    })?;

    Ok(instance_pre)
}
