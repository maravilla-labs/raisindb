// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What the runtime refuses to LOAD, as opposed to what it refuses to let run.
//!
//! All of these go through `FunctionRuntime::validate`, which is the same
//! `compile()` the execution path takes on a cache miss — so an artifact that
//! validates here cannot be rejected later, and one rejected here can never
//! reach a store. A7 hangs upload-time validation off the same function for
//! exactly that reason.

use super::*;
use crate::runtime::FunctionRuntime;
use crate::types::{FunctionCode, DEFAULT_MAX_WASM_ARTIFACT_BYTES};

const ECHO: &[u8] = include_bytes!("fixtures/echo.wasm");
const WRONG_WORLD: &[u8] = include_bytes!("fixtures/wrong_world.wasm");
const SOCKETS: &[u8] = include_bytes!("fixtures/sockets.wasm");

#[test]
fn an_artifact_over_the_cap_is_refused_before_anything_is_compiled() {
    let oversized = FunctionCode::from(vec![0u8; DEFAULT_MAX_WASM_ARTIFACT_BYTES + 1]);
    let error = WasmRuntime::new()
        .validate(&oversized)
        .unwrap_err()
        .to_string();
    assert!(error.contains("over the"), "{error}");
}

#[test]
fn a_core_module_is_not_a_component() {
    let core = wat::parse_str("(module)").unwrap();
    let error = WasmRuntime::new()
        .validate(&FunctionCode::from(core))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("not a valid WebAssembly component"),
        "{error}"
    );
}

#[test]
fn a_component_built_against_another_world_is_refused() {
    let error = WasmRuntime::new()
        .validate(&FunctionCode::from(WRONG_WORLD.to_vec()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not export"), "{error}");
    assert!(error.contains("handler"), "{error}");
}

#[test]
fn an_unlinked_wasi_sockets_import_is_refused_by_name() {
    // This is the egress boundary. Nothing downstream re-checks it, so the
    // rejection has to happen here — and has to say what it rejected.
    let error = WasmRuntime::new()
        .validate(&FunctionCode::from(SOCKETS.to_vec()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("wasi:sockets"), "{error}");
}

#[test]
fn a_valid_component_validates() {
    WasmRuntime::new()
        .validate(&FunctionCode::from(ECHO.to_vec()))
        .expect("the echo fixture must be accepted");
}

/// Cranelift must never run on a tokio worker.
///
/// This is the one claim in the upload path that reading the code cannot
/// settle: `validate_component_async` and `validate_component` compile the
/// same bytes and return the same answer, so a caller that picked the wrong
/// one is correct and silently starves the runtime. The difference is only
/// observable as *scheduling*, so that is what this measures.
///
/// The runtime has exactly ONE worker. A cooperative task counts how many
/// times it is scheduled; the measurement runs in a SECOND spawned task, so
/// both live on that one worker. A compile that runs on the worker freezes the
/// counter; a compile handed to `spawn_blocking` leaves it free. Both tasks
/// must be spawned — a `#[tokio::test(flavor = "multi_thread")]` body runs on
/// the main thread via `block_on`, so measuring from there would block a
/// thread the counter never uses and the test would pass no matter what.
///
/// The synchronous form is exercised as a negative control in the same
/// runtime: without it, a test that had stopped discriminating would still be
/// green.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn validating_an_artifact_does_not_park_the_only_worker() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc as StdArc;

    // Compile the same artifact several times per arm: one 44 KiB component is
    // milliseconds of Cranelift, and the gap should not rest on a single one.
    const ROUNDS: usize = 5;

    let ticks = StdArc::new(AtomicU64::new(0));
    let counter = {
        let ticks = ticks.clone();
        tokio::spawn(async move {
            loop {
                ticks.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        })
    };

    let measured = {
        let ticks = ticks.clone();
        tokio::spawn(async move {
            let bytes: StdArc<[u8]> = StdArc::from(ECHO.to_vec());
            // Let the counter reach its loop before either arm starts.
            tokio::task::yield_now().await;

            let before_async = ticks.load(Ordering::Relaxed);
            for _ in 0..ROUNDS {
                validate_component_async(bytes.clone())
                    .await
                    .expect("echo.wasm validates");
            }
            let during_async = ticks.load(Ordering::Relaxed) - before_async;

            // Negative control: the synchronous form, on the worker itself.
            let before_sync = ticks.load(Ordering::Relaxed);
            for _ in 0..ROUNDS {
                validate_component(&bytes).expect("echo.wasm validates");
            }
            let during_sync = ticks.load(Ordering::Relaxed) - before_sync;

            (during_async, during_sync)
        })
    };

    let (during_async, during_sync) = measured.await.expect("measurement task panicked");
    counter.abort();
    // Visible under --nocapture: the ratio is the evidence, not the pass.
    eprintln!(
        "worker scheduling during {ROUNDS} compiles: offloaded={during_async} ticks, \
         blocking={during_sync} ticks"
    );

    // The blocking form holds the worker inside Cranelift for the whole loop
    // and awaits nothing, so the counter cannot advance at all. Assert a small
    // ceiling rather than exactly 0 so the test is not hostage to scheduler
    // bookkeeping.
    assert!(
        during_sync <= 2,
        "the synchronous form was expected to park the worker, but the counter \
         advanced {during_sync} times — this test can no longer tell a blocking \
         compile from an offloaded one, so its green is meaningless"
    );

    // The offloaded form leaves the worker free. The counter is a tight
    // yield_now loop, so it advances thousands of times across a few
    // milliseconds of compile; require far less than that to stay clear of
    // slow or loaded machines, but far more than the control.
    assert!(
        during_async >= 50,
        "validate_component_async parked the only worker: the counter advanced \
         {during_async} times during {ROUNDS} offloaded compiles vs {during_sync} \
         during the same number of blocking ones. Cranelift is on a tokio worker."
    );
}
