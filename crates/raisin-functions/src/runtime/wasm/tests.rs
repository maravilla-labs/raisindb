// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for the WebAssembly runtime.
//!
//! Every fixture is a REAL component, built from `fixtures/wasm-guests` and
//! committed next door (see that directory's README). Nothing here is mocked
//! below the wasmtime boundary: a test that passed against a stubbed engine
//! would prove nothing about the linker subset, the limiter or the traps,
//! which is the entire surface this runtime adds.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::api::MockFunctionApi;
use crate::runtime::FunctionRuntime;
use crate::types::{
    ExecutionContext, ExecutionResult, FunctionCode, FunctionMetadata, ResourceLimits,
};

const ECHO: &[u8] = include_bytes!("fixtures/echo.wasm");
const CALL_HOST: &[u8] = include_bytes!("fixtures/call_host.wasm");
const LOG: &[u8] = include_bytes!("fixtures/log.wasm");
const SPIN: &[u8] = include_bytes!("fixtures/spin.wasm");
const ALLOC: &[u8] = include_bytes!("fixtures/alloc.wasm");

fn metadata(timeout_ms: u64, max_memory_bytes: u64) -> FunctionMetadata {
    FunctionMetadata::wasm("test_wasm_function").with_resource_limits(ResourceLimits {
        timeout_ms,
        max_memory_bytes,
        max_instructions: None,
        max_stack_bytes: 1024 * 1024,
    })
}

fn mock_api(context: serde_json::Value) -> Arc<MockFunctionApi> {
    Arc::new(MockFunctionApi::new(context))
}

/// Run one artifact under a handler name, with a default 5 s / 64 MiB budget.
async fn run(
    artifact: &[u8],
    handler: &str,
    input: serde_json::Value,
    api: Arc<MockFunctionApi>,
) -> ExecutionResult {
    run_with(
        artifact,
        handler,
        input,
        api,
        metadata(5_000, 64 * 1024 * 1024),
        None,
    )
    .await
}

async fn run_with(
    artifact: &[u8],
    handler: &str,
    input: serde_json::Value,
    api: Arc<MockFunctionApi>,
    metadata: FunctionMetadata,
    log_emitter: Option<raisin_storage::LogEmitter>,
) -> ExecutionResult {
    let mut context =
        ExecutionContext::new("tenant1", "repo1", "main", "test-user").with_input(input);
    context.log_emitter = log_emitter;

    WasmRuntime::new()
        .execute(
            &FunctionCode::from(artifact.to_vec()),
            handler,
            context,
            &metadata,
            api,
            HashMap::new(),
        )
        .await
        .expect("the runtime itself must not fail")
}

// ---------------------------------------------------------------------------
// Handler routing: one artifact, N handlers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_default_handler_runs_and_sees_its_input() {
    let result = run(
        ECHO,
        "default",
        json!({"message": "hello"}),
        mock_api(json!({})),
    )
    .await;

    assert!(result.success, "{:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["handler"], "default");
    assert_eq!(output["echo"]["message"], "hello");
    assert_eq!(output["abi"], super::HOST_ABI_VERSION);
}

#[tokio::test]
async fn one_artifact_serves_two_handlers() {
    // The whole point of the name-routed export: two Function nodes may share
    // this artifact and select different behaviour with `main.wasm:<handler>`.
    let default = run(ECHO, "default", json!({"n": 1}), mock_api(json!({}))).await;
    let reverse = run(ECHO, "reverse", json!({"n": 1}), mock_api(json!({}))).await;

    assert_eq!(default.output.unwrap()["handler"], "default");
    assert_eq!(reverse.output.unwrap()["handler"], "reverse");
}

#[tokio::test]
async fn an_unknown_handler_is_the_guests_error_naming_what_it_registered() {
    // The host must NOT hold an allow-list of handler names; the guest owns
    // its namespace and is the only thing that can answer this correctly.
    let result = run(ECHO, "main", json!({}), mock_api(json!({}))).await;

    assert!(!result.success);
    let error = result.error.unwrap();
    assert_eq!(error.code, "RUNTIME_ERROR");
    assert!(
        error.message.contains("unknown handler 'main'")
            && error.message.contains("default")
            && error.message.contains("reverse"),
        "the guest must name its registered handlers: {}",
        error.message
    );
}

// ---------------------------------------------------------------------------
// The host gateway
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_gateway_reaches_the_registry_and_reports_an_unknown_method_as_err() {
    let api = mock_api(json!({"tenant_id": "tenant1", "repo_id": "repo1"}));
    let result = run(CALL_HOST, "default", json!({}), api.clone()).await;

    assert!(result.success, "{:?}", result.error);
    let output = result.output.unwrap();

    // `context_get` came back through the same generic gateway as everything else.
    assert_eq!(output["context"]["tenant_id"], "tenant1");

    // `http_request` really reached the API: the mock recorded the exact call.
    let calls = api.http_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["method"], "GET");
    assert_eq!(calls[0]["url"], "https://example.com/thing");
    assert_eq!(calls[0]["options"]["headers"]["x-from"], "wasm");

    // An unknown method is an `Err` the guest can handle, never a trap.
    assert_eq!(
        output["unknown_error"],
        "Unknown raisin API method: no_such_method"
    );
}

#[tokio::test]
async fn two_executions_of_one_artifact_see_their_own_context() {
    // The compiled artifact is the ONLY thing shared across tenants, and it
    // holds no state: each store carries its own context JSON.
    let first = run(
        CALL_HOST,
        "default",
        json!({}),
        mock_api(json!({"tenant_id": "tenant-a"})),
    )
    .await;
    let second = run(
        CALL_HOST,
        "default",
        json!({}),
        mock_api(json!({"tenant_id": "tenant-b"})),
    )
    .await;

    assert_eq!(first.output.unwrap()["context"]["tenant_id"], "tenant-a");
    assert_eq!(second.output.unwrap()["context"]["tenant_id"], "tenant-b");
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_log_import_and_stdio_both_reach_the_result_and_the_emitter() {
    let emitted = Arc::new(AtomicUsize::new(0));
    let counter = emitted.clone();
    let emitter: raisin_storage::LogEmitter = Arc::new(move |_level, _message| {
        counter.fetch_add(1, Ordering::SeqCst);
    });

    let result = run_with(
        LOG,
        "default",
        json!({}),
        mock_api(json!({})),
        metadata(5_000, 64 * 1024 * 1024),
        Some(emitter),
    )
    .await;

    assert!(result.success, "{:?}", result.error);
    let messages: Vec<&str> = result.logs.iter().map(|l| l.message.as_str()).collect();
    assert!(messages.contains(&"info from wasm"), "{messages:?}");
    assert!(messages.contains(&"error from wasm"), "{messages:?}");
    assert!(messages.contains(&"stdout from wasm"), "{messages:?}");
    assert!(messages.contains(&"stderr from wasm"), "{messages:?}");

    // Only the `log` import streams live; stdout/stderr are drained at the end.
    assert_eq!(emitted.load(Ordering::SeqCst), 2);
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_spinning_guest_is_cut_off_by_the_epoch_deadline() {
    let started = std::time::Instant::now();
    let result = run_with(
        SPIN,
        "default",
        json!({}),
        mock_api(json!({})),
        metadata(200, 64 * 1024 * 1024),
        None,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(!result.success);
    assert_eq!(result.error.unwrap().code, "TIMEOUT");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "the deadline must fire promptly, took {elapsed:?}"
    );
}

#[tokio::test]
async fn a_guest_over_its_memory_budget_is_reported_as_such() {
    // The guest allocator aborts when `memory.grow` is refused, so the trap
    // says `unreachable`; only the limiter knows it was the cap.
    let result = run_with(
        ALLOC,
        "default",
        json!({}),
        mock_api(json!({})),
        metadata(30_000, 32 * 1024 * 1024),
        None,
    )
    .await;

    assert!(!result.success);
    assert_eq!(result.error.unwrap().code, "MEMORY_LIMIT");
}

// ---------------------------------------------------------------------------
// AssemblyScript
// ---------------------------------------------------------------------------

/// A component produced by AssemblyScript, not by a first-party SDK.
///
/// AssemblyScript deliberately does not implement WASI or the Component Model
/// and has no maintained `wit-bindgen` backend, so `asc` emits a CORE MODULE.
/// This fixture is that module, hand-lowered to the canonical ABI and wrapped
/// with `wasm-tools component embed` + `component new` — the path
/// `docs/design/assemblyscript-guest.md` proposes for a third guest language.
///
/// The test exists to keep that path honest: if the host ever stops accepting
/// a component built this way, an AssemblyScript SDK built on it is dead, and
/// the reason should surface here rather than in someone's scaffold.
///
/// Regenerate: see `fixtures/wasm-guests/assemblyscript/README.md`.
const ASSEMBLYSCRIPT: &[u8] = include_bytes!("fixtures/assemblyscript.wasm");

#[tokio::test]
async fn an_assemblyscript_component_uses_the_host_imports() {
    let api = mock_api(json!({"tenant_id": "tenant1"}));
    let result = run(ASSEMBLYSCRIPT, "greet", json!({"name": "Ada"}), api.clone()).await;

    assert!(result.success, "{:?}", result.error);
    let output = result.output.unwrap();

    // EXPORT side: both canonical-ABI string parameters arrived intact. A wrong
    // pointer or length corrupts one of these rather than failing loudly.
    assert_eq!(output["handler"], "greet");
    assert_eq!(output["echo"]["name"], "Ada");

    // IMPORT side, the half where ABI mistakes are silent:
    // `context()` returns a string indirectly through a guest-owned area.
    assert_eq!(output["ctx_has_tenant"], true, "context() came back empty");

    // `call()` returns result<string, string> as (tag, ptr, len); the guest
    // reports the tag and the decoded body, so a mis-read discriminant shows up
    // as call_ok=false rather than as plausible-looking data.
    assert_eq!(output["call_ok"], true, "call() reported an error tag");
    assert!(
        output["children"].is_array(),
        "call() body did not decode as the mock's node list: {}",
        output["children"]
    );

    // `log()` takes an enum plus a string and returns nothing.
    let logs = result
        .logs
        .iter()
        .map(|l| l.message.clone())
        .collect::<Vec<_>>();
    assert!(
        logs.iter()
            .any(|m| m.contains("assemblyscript guest running")),
        "log() did not reach the execution logs: {logs:?}"
    );
}

/// A guest produced by `raisindb create function --lang assemblyscript`,
/// unmodified, built by `raisindb function build`.
///
/// This is what an AssemblyScript user actually writes: `nodes.getChildren(...)`
/// from `assembly/generated.ts`, which the binding generator emits from the same
/// registry the Rust and Go SDKs come from. It exists separately from the
/// hand-lowered fixture because the two can fail independently — the ABI can be
/// right while the generated argument encoding is wrong, and vice versa.
///
/// Regenerate: scaffold it, `npm install`, `raisindb function build`, and copy
/// the artifact here. That is deliberately the USER's path — if the scaffold
/// stops producing something the host accepts, this test is where it shows.
const ASSEMBLYSCRIPT_SDK: &[u8] = include_bytes!("fixtures/assemblyscript_sdk.wasm");

#[tokio::test]
async fn the_assemblyscript_sdk_surface_reaches_the_host() {
    let result = run(
        ASSEMBLYSCRIPT_SDK,
        "default",
        json!({"name": "Ada"}),
        mock_api(json!({"tenant_id": "tenant1"})),
    )
    .await;

    assert!(result.success, "{:?}", result.error);
    let output = result.output.unwrap();

    // `nodes.getChildren` went through the generated wrapper, which builds the
    // positional JSON argument array itself.
    assert_eq!(output["greeting"], "hello");

    // `nodes.getChildren` went through the generated wrapper, which builds the
    // positional JSON argument array itself — a wrong encoding here surfaces as
    // an argument error from the host rather than as a decode failure.
    assert!(
        output["children"].is_array(),
        "generated nodes.getChildren did not return the mock's node list: {}",
        output["children"]
    );
    assert_eq!(output["children"][0]["node_type"], "raisin:Page");

    assert!(
        result
            .logs
            .iter()
            .any(|l| l.message.contains("greet-as running")),
        "log.info from the SDK did not reach the execution logs"
    );
}
