// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Compile-cache behaviour: hit, single flight, cap, and cross-tenant sharing.
//!
//! # Why every test here uses its own artifact
//!
//! The cache and its counters are process-wide, and `cargo test` runs this
//! binary's tests in parallel — a test that asserted on the GLOBAL compile
//! count would be measuring whatever else happened to compile at the same
//! moment. So each test appends a custom section with a unique name to a real
//! fixture: the result is a byte-different but still perfectly valid component,
//! which is exactly the situation the content-addressed key is designed for,
//! and `compile_count_of` then answers about that artifact alone.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;

use super::cache::{check_artifact_cap, compile_count, compile_count_of, get_or_compile};
use super::WasmRuntime;
use crate::api::MockFunctionApi;
use crate::runtime::FunctionRuntime;
use crate::types::{
    ExecutionContext, ExecutionResult, FunctionCode, FunctionMetadata, ResourceLimits,
    DEFAULT_MAX_WASM_ARTIFACT_BYTES,
};

const ECHO: &[u8] = include_bytes!("fixtures/echo.wasm");
const CALL_HOST: &[u8] = include_bytes!("fixtures/call_host.wasm");

/// A byte-distinct but still valid copy of a component.
///
/// Appends a custom section (id 0, `name`-prefixed payload) — legal anywhere at
/// a component's top level and ignored by the validator, so the artifact still
/// compiles while hashing to a fresh cache key.
fn uniquified(base: &[u8], tag: &str) -> Vec<u8> {
    let name = format!("raisin-test-{tag}");
    assert!(name.len() < 0x80, "the single-byte LEB shortcut needs it");

    let mut payload = vec![name.len() as u8];
    payload.extend_from_slice(name.as_bytes());
    assert!(payload.len() < 0x80, "ditto for the section size");

    let mut out = base.to_vec();
    out.push(0x00); // custom section
    out.push(payload.len() as u8);
    out.extend_from_slice(&payload);
    out
}

fn metadata() -> FunctionMetadata {
    FunctionMetadata::wasm("test_wasm_cache").with_resource_limits(ResourceLimits {
        timeout_ms: 10_000,
        max_memory_bytes: 64 * 1024 * 1024,
        max_instructions: None,
        max_stack_bytes: 1024 * 1024,
    })
}

async fn run(artifact: &[u8], context: serde_json::Value) -> ExecutionResult {
    WasmRuntime::new()
        .execute(
            &FunctionCode::from(artifact.to_vec()),
            "default",
            ExecutionContext::new("tenant1", "repo1", "main", "test-user").with_input(json!({})),
            &metadata(),
            Arc::new(MockFunctionApi::new(context)),
            HashMap::new(),
        )
        .await
        .expect("the runtime itself must not fail")
}

// ---------------------------------------------------------------------------
// Hit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_second_execution_of_the_same_artifact_does_not_recompile() {
    let artifact = uniquified(ECHO, "cache-hit");
    assert_eq!(compile_count_of(&artifact), 0, "must start cold");

    let before_total = compile_count();
    for _ in 0..4 {
        assert!(run(&artifact, json!({})).await.success);
    }

    assert_eq!(
        compile_count_of(&artifact),
        1,
        "four executions of one artifact are one compile"
    );
    assert!(
        compile_count() > before_total,
        "the process-wide counter must have moved too"
    );
}

#[tokio::test]
async fn a_hit_hands_back_the_same_compiled_component() {
    let artifact: Arc<[u8]> = Arc::from(uniquified(ECHO, "hit-identity"));

    let first = get_or_compile(artifact.clone()).await.unwrap();
    let second = get_or_compile(artifact.clone()).await.unwrap();

    assert!(
        Arc::ptr_eq(&first, &second),
        "a hit must return the cached entry, not an equal-looking new one"
    );
    assert!(first.image_bytes > 0, "the weigher must see a real size");
    assert_eq!(first.artifact_bytes, artifact.len());
}

// ---------------------------------------------------------------------------
// Single flight
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_concurrent_cold_executions_compile_once() {
    // The deploy burst: every caller arrives before anyone has finished
    // compiling. Without the per-key lock this is eight Cranelift runs.
    let artifact = Arc::new(uniquified(ECHO, "single-flight"));
    assert_eq!(compile_count_of(&artifact), 0, "must start cold");

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let artifact = artifact.clone();
        tasks.push(tokio::spawn(async move {
            run(&artifact, json!({})).await.success
        }));
    }
    for task in tasks {
        assert!(task.await.expect("task joins"), "every caller must succeed");
    }

    assert_eq!(
        compile_count_of(&artifact),
        1,
        "eight concurrent cold callers must produce exactly one compile"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_cold_callers_all_receive_one_entry() {
    // The same property seen from the other side: had two of them compiled,
    // they would be holding two different `Arc`s.
    let artifact: Arc<[u8]> = Arc::from(uniquified(ECHO, "single-flight-identity"));

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let artifact = artifact.clone();
        tasks.push(tokio::spawn(async move {
            get_or_compile(artifact).await.unwrap()
        }));
    }
    let mut entries = Vec::new();
    for task in tasks {
        entries.push(task.await.expect("task joins"));
    }

    for entry in &entries[1..] {
        assert!(
            Arc::ptr_eq(&entries[0], entry),
            "all eight must share one compiled component"
        );
    }
}

// ---------------------------------------------------------------------------
// The artifact cap
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_oversized_artifact_never_reaches_the_compiler() {
    let oversized: Arc<[u8]> = Arc::from(vec![0u8; DEFAULT_MAX_WASM_ARTIFACT_BYTES + 1]);

    let error = get_or_compile(oversized.clone())
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("over the"), "{error}");
    assert!(
        error.contains(&DEFAULT_MAX_WASM_ARTIFACT_BYTES.to_string()),
        "{error}"
    );
    // Per artifact, not the process-wide counter: another test compiling in
    // parallel would otherwise make this assertion about the wrong thing.
    assert_eq!(
        compile_count_of(&oversized),
        0,
        "the cap is checked before anything is handed to Cranelift"
    );
}

#[test]
fn the_cap_is_one_check_shared_by_every_gate() {
    check_artifact_cap(DEFAULT_MAX_WASM_ARTIFACT_BYTES).expect("exactly at the cap is allowed");
    assert!(check_artifact_cap(DEFAULT_MAX_WASM_ARTIFACT_BYTES + 1).is_err());
}

// ---------------------------------------------------------------------------
// Isolation: one artifact, two tenants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_tenants_share_one_compiled_artifact_and_see_their_own_context() {
    // The cache entry is the ONLY thing crossing the tenant boundary here, and
    // it is code. If any state rode along with it, the second tenant would see
    // the first tenant's context.
    let artifact = uniquified(CALL_HOST, "two-tenants");

    let a = run(&artifact, json!({"tenant_id": "tenant-a"})).await;
    let b = run(&artifact, json!({"tenant_id": "tenant-b"})).await;

    assert!(a.success, "{:?}", a.error);
    assert!(b.success, "{:?}", b.error);
    assert_eq!(a.output.unwrap()["context"]["tenant_id"], "tenant-a");
    assert_eq!(b.output.unwrap()["context"]["tenant_id"], "tenant-b");

    assert_eq!(
        compile_count_of(&artifact),
        1,
        "byte-identical artifacts are one compile however many tenants run them"
    );
}
