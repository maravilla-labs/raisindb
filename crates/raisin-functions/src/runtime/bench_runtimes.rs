// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Cross-runtime benchmark: QuickJS vs Starlark vs WebAssembly.
//!
//! `cargo test -p raisin-functions --lib --features wasm bench_runtimes -- --ignored --nocapture`
//!
//! # What this measures, and what it deliberately does not
//!
//! The three arms run against [`MockFunctionApi`], so a host call returns
//! canned data instead of touching RocksDB. That is on purpose: the storage
//! work behind `nodes.getChildren` is byte-for-byte the same whichever runtime
//! asked for it, so including it would add a large constant to every arm and
//! bury the thing being compared. What is left is exactly the part that
//! differs — parse/compile, instantiate, guest execution, and the JSON
//! marshalling of each gateway round-trip.
//!
//! An end-to-end number, with real SQL against a real server, is a different
//! measurement and belongs in `raisin-server`'s integration tests.
//!
//! # Why four workloads
//!
//! A single number would hide the shape. These decompose it:
//!
//! | workload    | isolates                                          |
//! |-------------|---------------------------------------------------|
//! | `noop`      | dispatch floor — compile/parse, instantiate, call  |
//! | `compute`   | guest execution speed, zero host calls            |
//! | `hostcalls` | marshalling — `n` identical gateway round-trips   |
//! | `realistic` | one node read + one SQL query, results aggregated |
//!
//! # The correctness guard
//!
//! Every arm's output is asserted EQUAL before any timing is reported. Three
//! runtimes that disagree are timing different work, and the table would be
//! meaningless — so the benchmark fails rather than prints.

use crate::api::MockFunctionApi;
use crate::runtime::{FunctionRuntime, QuickJsRuntime, StarlarkRuntime, WasmRuntime};
use crate::types::{ExecutionContext, FunctionCode, FunctionMetadata, ResourceLimits};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const BENCH_WASM: &[u8] = include_bytes!("wasm/fixtures/bench.wasm");

/// Iterations for the `compute` loop. Kept modest because Starlark walks an
/// AST per iteration; raise it to widen the gap, not to change the ranking.
const COMPUTE_N: u64 = 100_000;
/// Gateway round-trips for the `hostcalls` workload.
const HOSTCALL_N: u64 = 200;
/// Steady-state samples taken after the cold run.
const WARM_SAMPLES: usize = 20;

/// The JavaScript and Starlark arms live in `fixtures/bench-sources/` because
/// `raisin-server`'s end-to-end benchmark runs the SAME sources against a real
/// server. Two copies would drift and the two benchmarks would quietly stop
/// comparing the same work.
const JS_SOURCE: &str = include_str!("../../../../fixtures/bench-sources/bench.js");
const STARLARK_SOURCE: &str = include_str!("../../../../fixtures/bench-sources/bench.star");

/// One runtime under test.
struct Arm {
    label: &'static str,
    runtime: Box<dyn FunctionRuntime>,
    code: FunctionCode,
    metadata: FunctionMetadata,
}

fn limits() -> ResourceLimits {
    ResourceLimits {
        // Generous: Starlark's `compute` arm is an AST walk and must not be
        // timed out mid-measurement.
        timeout_ms: 120_000,
        max_memory_bytes: 256 * 1024 * 1024,
        max_instructions: None,
        max_stack_bytes: 1024 * 1024,
    }
}

fn arms() -> Vec<Arm> {
    vec![
        Arm {
            label: "quickjs",
            runtime: Box::new(QuickJsRuntime::new()),
            code: FunctionCode::from(JS_SOURCE),
            metadata: FunctionMetadata::javascript("bench").with_resource_limits(limits()),
        },
        Arm {
            label: "starlark",
            runtime: Box::new(StarlarkRuntime::new()),
            code: FunctionCode::from(STARLARK_SOURCE),
            metadata: FunctionMetadata::starlark("bench").with_resource_limits(limits()),
        },
        Arm {
            label: "wasm",
            runtime: Box::new(WasmRuntime::new()),
            code: FunctionCode::from(BENCH_WASM.to_vec()),
            metadata: FunctionMetadata::wasm("bench").with_resource_limits(limits()),
        },
    ]
}

async fn invoke(
    arm: &Arm,
    handler: &str,
    input: serde_json::Value,
) -> (serde_json::Value, Duration) {
    let context = ExecutionContext::new("tenant1", "repo1", "main", "bench-user").with_input(input);
    let api: Arc<dyn crate::api::FunctionApi> = Arc::new(MockFunctionApi::new(
        serde_json::json!({"tenant_id": "tenant1"}),
    ));

    let started = Instant::now();
    let result = arm
        .runtime
        .execute(
            &arm.code,
            handler,
            context,
            &arm.metadata,
            api,
            HashMap::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("[{}] {handler} failed to run: {e}", arm.label));
    let elapsed = started.elapsed();

    assert!(
        result.success,
        "[{}] {handler} returned an error: {:?}",
        arm.label, result.error
    );
    (result.output.unwrap_or(serde_json::Value::Null), elapsed)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Cold run, then `WARM_SAMPLES` steady-state runs, for one workload.
async fn measure(workload: &str, handler: &str, input: serde_json::Value, rows: &mut Vec<String>) {
    let arms = arms();
    let mut outputs: Vec<(&str, serde_json::Value)> = Vec::new();
    let mut timings: Vec<(&str, f64, f64, f64)> = Vec::new();

    for arm in &arms {
        // COLD: first invocation of this artifact in this process — for wasm
        // it pays the Cranelift compile, for QuickJS the parse and a pool
        // runtime, for Starlark the AST build.
        let (output, cold) = invoke(arm, handler, input.clone()).await;

        let mut samples = Vec::with_capacity(WARM_SAMPLES);
        for _ in 0..WARM_SAMPLES {
            let (again, warm) = invoke(arm, handler, input.clone()).await;
            assert_eq!(
                again, output,
                "[{}] {workload} is not deterministic across runs",
                arm.label
            );
            samples.push(ms(warm));
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = samples[samples.len() / 2];
        let best = samples[0];

        outputs.push((arm.label, output));
        timings.push((arm.label, ms(cold), median, best));
    }

    // The guard: three runtimes that disagree are not comparable.
    let (first_label, first_output) = &outputs[0];
    for (label, output) in &outputs[1..] {
        assert_eq!(
            output, first_output,
            "{workload}: {label} produced a different answer than {first_label} — \
             the arms are not running the same workload, so timing them is meaningless"
        );
    }

    let baseline = timings
        .iter()
        .map(|(_, _, median, _)| *median)
        .fold(f64::INFINITY, f64::min);

    rows.push(format!(
        "\n  {workload}  (output: {})",
        serde_json::to_string(first_output).unwrap_or_default()
    ));
    rows.push(format!(
        "    {:<10} {:>12} {:>12} {:>12} {:>8}",
        "runtime", "cold ms", "median ms", "best ms", "vs best"
    ));
    for (label, cold, median, best) in &timings {
        rows.push(format!(
            "    {:<10} {:>12.3} {:>12.3} {:>12.3} {:>7.1}x",
            label,
            cold,
            median,
            best,
            median / baseline
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
async fn compare_quickjs_starlark_and_wasm() {
    let mut rows = vec![format!(
        "\n=== runtime comparison ({WARM_SAMPLES} warm samples, mock host) ==="
    )];

    measure("noop", "noop", serde_json::json!({}), &mut rows).await;
    measure(
        &format!("compute (n={COMPUTE_N})"),
        "compute",
        serde_json::json!({ "n": COMPUTE_N }),
        &mut rows,
    )
    .await;
    measure(
        &format!("hostcalls (n={HOSTCALL_N})"),
        "hostcalls",
        serde_json::json!({ "n": HOSTCALL_N }),
        &mut rows,
    )
    .await;
    measure("realistic", "realistic", serde_json::json!({}), &mut rows).await;

    eprintln!("{}", rows.join("\n"));
}
