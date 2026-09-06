// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Turning a wasmtime failure into an `ExecutionError` a tenant may read.
//!
//! The classification is not cosmetic. A refused `memory.grow` reaches us as
//! `unreachable` (the guest allocator aborts), and a deadline reaches us as
//! `Trap::Interrupt` — both would otherwise be reported as an anonymous
//! runtime error, and an operator would have no way to tell "the function is
//! broken" from "the function needs a bigger limit".
//!
//! Backtraces are GUEST frames only (`wasmtime::WasmBacktrace`); the host
//! stack is never rendered into a tenant-visible message.

use crate::types::ExecutionError;

use super::limits::FunctionLimiter;

/// Classify a failed guest call.
///
/// `limiter` must be the one the store used — the memory verdict is read from
/// it, not from the trap.
pub fn classify_trap(
    err: &wasmtime::Error,
    limiter: &FunctionLimiter,
    timeout_ms: u64,
) -> ExecutionError {
    let trap = err.downcast_ref::<wasmtime::Trap>().copied();

    let mut execution_error = match trap {
        Some(wasmtime::Trap::Interrupt) => ExecutionError::timeout(timeout_ms),
        Some(wasmtime::Trap::StackOverflow) => ExecutionError::new(
            "STACK_OVERFLOW",
            "Function exceeded the WebAssembly stack limit",
        ),
        _ if limiter.denied_memory_growth() => {
            ExecutionError::memory_limit(limiter.max_memory_bytes() as u64)
        }
        Some(t) => ExecutionError::runtime(format!("wasm trap: {t}")),
        None => ExecutionError::runtime(format!("wasm execution failed: {err:#}")),
    };

    if let Some(backtrace) = err.downcast_ref::<wasmtime::WasmBacktrace>() {
        let rendered = backtrace.to_string();
        if !rendered.is_empty() {
            execution_error = execution_error.with_stack_trace(rendered);
        }
    }

    execution_error
}

/// The failure for an execution that never produced a result because the outer
/// wall clock ran out. Distinct from `Trap::Interrupt` only in provenance: an
/// epoch cuts off GUEST code, this one also covers a host call that hung.
pub fn outer_timeout(timeout_ms: u64) -> ExecutionError {
    ExecutionError::timeout(timeout_ms)
}

/// The guest returned `Ok`, but the payload is not JSON. The contract says the
/// Ok side of `handler` is JSON output, so this is a guest bug and is reported
/// as one rather than being wrapped in a string.
pub fn invalid_output(err: &serde_json::Error, output: &str) -> ExecutionError {
    let preview: String = output.chars().take(200).collect();
    ExecutionError::new(
        "INVALID_OUTPUT",
        format!("Function returned invalid JSON ({err}): {preview}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::ResourceLimiter;

    #[test]
    fn an_interrupt_is_a_timeout() {
        let limiter = FunctionLimiter::new(1024);
        let err = wasmtime::Error::new(wasmtime::Trap::Interrupt);
        assert_eq!(classify_trap(&err, &limiter, 250).code, "TIMEOUT");
    }

    #[test]
    fn a_denied_growth_outranks_the_unreachable_it_caused() {
        // The guest allocator aborts when `memory.grow` returns -1, so the trap
        // we see says `unreachable`. Only the limiter knows what happened.
        let mut limiter = FunctionLimiter::new(1024);
        let _ = limiter.memory_growing(0, 1 << 30, None);
        let err = wasmtime::Error::new(wasmtime::Trap::UnreachableCodeReached);
        assert_eq!(classify_trap(&err, &limiter, 250).code, "MEMORY_LIMIT");
    }

    #[test]
    fn an_ordinary_trap_stays_a_runtime_error() {
        let limiter = FunctionLimiter::new(1024);
        let err = wasmtime::Error::new(wasmtime::Trap::UnreachableCodeReached);
        let classified = classify_trap(&err, &limiter, 250);
        assert_eq!(classified.code, "RUNTIME_ERROR");
        assert!(classified.message.contains("wasm trap"));
    }

    #[test]
    fn a_stack_overflow_is_named() {
        let limiter = FunctionLimiter::new(1024);
        let err = wasmtime::Error::new(wasmtime::Trap::StackOverflow);
        assert_eq!(classify_trap(&err, &limiter, 250).code, "STACK_OVERFLOW");
    }
}
