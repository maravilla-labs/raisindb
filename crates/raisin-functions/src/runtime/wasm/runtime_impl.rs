// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `FunctionRuntime` for WebAssembly components.
//!
//! # Tenant isolation
//!
//! The QuickJS pool keeps seven invariants (`quickjs/pool.rs:15-47`) because a
//! *runtime* is reused across tenants. Here a **`Store` is built fresh for
//! every execution and dropped at the end of it**, which satisfies the four
//! that apply — exclusive use (1), no globals carried forward (2), limits set
//! per execution (4), and no reuse after a failure (6) — structurally rather
//! than by discipline. Invariants 3, 5 and 7 exist only because a runtime is
//! recycled; nothing is recycled here.
//!
//! What IS shared across tenants is the compiled artifact, keyed by OUR hash
//! of its bytes (`cache.rs`) and never by the node's `content_hash`, which a
//! tenant can write. A compiled component holds no mutable state: code, not
//! data — so two tenants running byte-identical artifacts share one compiled
//! image and still see their own context, input and API.
//!
//! # Handler routing
//!
//! `entrypoint` arrives already resolved by `execution::entry_file` — the
//! suffix of `entry_file` (`main.wasm:on-order` -> `"on-order"`), or
//! `"default"` for a bare `main.wasm`. It is passed into the guest verbatim.
//! **Never validate it against a list**: the guest owns its handler namespace,
//! and answers an unknown name with an `Err` naming what it registered. A host
//! allow-list would have to be kept in sync with every SDK.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use raisin_error::{Error, Result};
use wasmtime::Store;
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};

use crate::api::FunctionApi;
use crate::runtime::FunctionRuntime;
use crate::types::{
    ExecutionContext, ExecutionResult, ExecutionStats, FunctionCode, FunctionLanguage,
    FunctionMetadata, LogEntry,
};

use super::bindings::{FunctionPre, HostState};
use super::cache::{check_artifact_cap, get_or_compile, validate_component};
use super::capture::CaptureStream;
use super::config::config;
use super::engine::engine;
use super::errors::{classify_trap, invalid_output, outer_timeout};
use super::limits::FunctionLimiter;

/// Grace period on top of the function's timeout for the outer wall clock.
/// Epoch interruption only cuts off GUEST code; a host call awaiting I/O is
/// not interrupted by it, so the outer timeout is the backstop.
const OUTER_TIMEOUT_GRACE: Duration = Duration::from_millis(250);

/// WebAssembly component runtime.
///
/// Zero-sized: the engine, linker and compile cache are process-wide statics,
/// so `new()` is free and may be called from any of the sites that build a
/// `RuntimeRegistry`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WasmRuntime;

impl WasmRuntime {
    /// Create a wasm runtime handle.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FunctionRuntime for WasmRuntime {
    async fn execute(
        &self,
        code: &FunctionCode,
        entrypoint: &str,
        context: ExecutionContext,
        metadata: &FunctionMetadata,
        api: Arc<dyn FunctionApi>,
        _files: HashMap<String, String>,
    ) -> Result<ExecutionResult> {
        let conf = config();
        if !conf.enabled {
            return Err(Error::Validation(
                "WebAssembly functions are disabled on this server ([functions.wasm] enabled = false)"
                    .to_string(),
            ));
        }
        // Cheap, and before the permit: an oversized artifact should not queue
        // behind fifteen running functions to be told no.
        check_artifact_cap(code.len())?;

        let start = std::time::Instant::now();
        let execution_id = context.execution_id.clone();
        let limits = &metadata.resource_limits;

        // Same permit as the two text runtimes. For wasm it bounds MEMORY (N
        // concurrent stores x max_memory_bytes), not worker threads: nothing
        // here calls `block_in_place`.
        let _permit = tokio::time::timeout(
            Duration::from_secs(60),
            crate::runtime::FUNCTION_EXECUTION_SEMAPHORE.acquire(),
        )
        .await
        .map_err(|_| {
            Error::Internal(
                "Function execution timed out waiting for available slot (all execution slots busy)"
                    .to_string(),
            )
        })?
        .map_err(|_| Error::Internal("Function execution semaphore closed".to_string()))?;

        tracing::debug!(
            execution_id = %execution_id,
            handler = %entrypoint,
            artifact_bytes = code.len(),
            "Executing wasm function"
        );

        // Compiled once per artifact, process-wide, keyed by our hash of the
        // bytes. A miss compiles inside `spawn_blocking` under a single-flight
        // lock; a hit is a refcount bump. See `cache.rs` for why the key can
        // never be the node's `content_hash`.
        let compiled = get_or_compile(artifact_bytes(code)).await?;

        let input_json = serde_json::to_string(&context.input)
            .map_err(|e| Error::Internal(format!("function input is not serializable: {e}")))?;
        let context_json = serde_json::to_string(&api.get_context())
            .map_err(|e| Error::Internal(format!("execution context is not serializable: {e}")))?;

        let stdout = CaptureStream::new(conf.stdout_capture_bytes);
        let stderr = CaptureStream::new(conf.stdout_capture_bytes);

        // No args, no env, no preopens: a function reaches the outside world
        // through `raisin.*` and nothing else.
        let wasi = WasiCtxBuilder::new()
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .build();

        let mut store = Store::new(
            engine()?,
            HostState {
                api,
                logs: Vec::new(),
                log_emitter: context.log_emitter.clone(),
                context_json,
                limiter: FunctionLimiter::new(limits.max_memory_bytes as usize),
                wasi,
                table: ResourceTable::new(),
            },
        );
        store.limiter(|state| &mut state.limiter);

        // One epoch tick is the timeout's resolution, so round UP and add one.
        let ticks = limits.timeout_ms.div_ceil(conf.epoch_tick_ms.max(1)) + 1;
        store.set_epoch_deadline(ticks);

        let deadline = Duration::from_millis(limits.timeout_ms) + OUTER_TIMEOUT_GRACE;
        let call = async {
            let instance = FunctionPre::new(compiled.instance_pre.clone())?
                .instantiate_async(&mut store)
                .await?;
            instance
                .call_handler(&mut store, entrypoint, &input_json)
                .await
        };
        let outcome = tokio::time::timeout(deadline, call).await;

        let stats = ExecutionStats {
            duration_ms: start.elapsed().as_millis() as u64,
            memory_used_bytes: store.data().limiter.peak_memory() as u64,
            ..Default::default()
        };

        let error = match outcome {
            // The guest returned JSON: the happy path.
            Ok(Ok(Ok(output))) => match serde_json::from_str::<serde_json::Value>(&output) {
                Ok(value) => {
                    let state = store.into_data();
                    let logs = drain_logs(state.logs, &stdout, &stderr);
                    return Ok(ExecutionResult::success(execution_id, value, stats).with_logs(logs));
                }
                Err(e) => invalid_output(&e, &output),
            },
            // The guest returned its own failure message.
            Ok(Ok(Err(message))) => crate::types::ExecutionError::runtime(message),
            // A trap, or instantiation failed.
            Ok(Err(e)) => classify_trap(&e, &store.data().limiter, limits.timeout_ms),
            // The wall clock ran out: a host call, not guest code, hung.
            Err(_) => outer_timeout(limits.timeout_ms),
        };

        tracing::warn!(
            execution_id = %execution_id,
            handler = %entrypoint,
            code = %error.code,
            message = %error.message,
            "wasm function failed"
        );

        let state = store.into_data();
        let logs = drain_logs(state.logs, &stdout, &stderr);
        Ok(ExecutionResult::failure(execution_id, error, stats).with_logs(logs))
    }

    fn validate(&self, code: &FunctionCode) -> Result<()> {
        // The same function the upload path and the package installer call, so
        // an artifact cannot pass one gate and fail another.
        validate_component(code.as_bytes())
    }

    fn language(&self) -> FunctionLanguage {
        FunctionLanguage::Wasm
    }

    fn name(&self) -> &'static str {
        "wasm"
    }
}

/// The artifact, without copying it when it already is one.
fn artifact_bytes(code: &FunctionCode) -> Arc<[u8]> {
    match code {
        FunctionCode::Bytes(bytes) => bytes.clone(),
        FunctionCode::Text(text) => Arc::from(text.as_bytes().to_vec()),
    }
}

/// Merge the `log` import's entries with whatever the guest printed.
///
/// stdout becomes `info` and stderr `error`, the same mapping `console.log` /
/// `console.error` get in QuickJS (`quickjs/console.rs`). Lines are split so a
/// multi-line print is many entries rather than one blob.
fn drain_logs(
    mut logs: Vec<LogEntry>,
    stdout: &CaptureStream,
    stderr: &CaptureStream,
) -> Vec<LogEntry> {
    for line in stdout.contents().lines() {
        if !line.is_empty() {
            logs.push(LogEntry::info(line));
        }
    }
    for line in stderr.contents().lines() {
        if !line.is_empty() {
            logs.push(LogEntry::error(line));
        }
    }
    logs
}
