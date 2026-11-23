// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The host-gateway contract, run against EVERY runtime.
//!
//! One function per language makes the same host call (`http_request`) to a
//! provider that never answers. Whatever the language, the deadline must cut
//! the call, a stop must cut it, a deterministic function must be refused, and
//! the failure must say which of those it was. Each runtime reaches the host
//! through `host_gateway.rs`; this is what keeps it that way.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::api::MockFunctionApi;
#[cfg(feature = "wasm")]
use crate::runtime::WasmRuntime;
use crate::runtime::{FunctionRuntime, QuickJsRuntime, StarlarkRuntime};
use crate::types::{
    ExecutionContext, ExecutionPolicy, ExecutionResult, FunctionCode, FunctionMetadata,
    ResourceLimits,
};

#[cfg(feature = "wasm")]
const CALL_HOST_WASM: &[u8] = include_bytes!("wasm/fixtures/call_host.wasm");

const JS: &str = r#"
function handler(input) {
    const r = raisin.http.request('GET', 'https://example.com/thing', {});
    return { visible: r && r.error ? r.error : JSON.stringify(r) };
}
"#;

const STARLARK: &str = r#"
def handler(input):
    return raisin.http.request("GET", "https://example.com/thing", {})
"#;

#[derive(Clone, Copy, Debug)]
enum Lang {
    JavaScript,
    Starlark,
    #[cfg(feature = "wasm")]
    Wasm,
}

fn limits(timeout_ms: u64) -> ResourceLimits {
    ResourceLimits {
        timeout_ms,
        max_memory_bytes: 64 * 1024 * 1024,
        max_instructions: None,
        max_stack_bytes: 1024 * 1024,
    }
}

/// Run the language's host-calling function; returns what a caller sees (the
/// output's message or the error's) and how long it took.
async fn run(
    lang: Lang,
    timeout_ms: u64,
    policy: ExecutionPolicy,
    execution_id: &str,
) -> (String, Duration) {
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})).with_http_hang());
    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_execution_id(execution_id)
        .with_policy(policy)
        .with_input(serde_json::json!({}));
    let started = Instant::now();
    let result: ExecutionResult = match lang {
        Lang::JavaScript => QuickJsRuntime::new()
            .execute(
                &FunctionCode::from(JS),
                "handler",
                context,
                &FunctionMetadata::javascript("contract").with_resource_limits(limits(timeout_ms)),
                api,
                HashMap::new(),
            )
            .await
            .expect("the runtime itself must not fail"),
        Lang::Starlark => StarlarkRuntime::new()
            .execute(
                &FunctionCode::from(STARLARK),
                "handler",
                context,
                &FunctionMetadata::starlark("contract").with_resource_limits(limits(timeout_ms)),
                api,
                HashMap::new(),
            )
            .await
            .expect("the runtime itself must not fail"),
        #[cfg(feature = "wasm")]
        Lang::Wasm => WasmRuntime::new()
            .execute(
                &FunctionCode::from(CALL_HOST_WASM.to_vec()),
                "default",
                context,
                &FunctionMetadata::wasm("contract").with_resource_limits(limits(timeout_ms)),
                api,
                HashMap::new(),
            )
            .await
            .expect("the runtime itself must not fail"),
    };
    let visible = match (&result.output, &result.error) {
        (_, Some(e)) => e.message.clone(),
        (Some(out), None) => out
            .get("visible")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| out.to_string()),
        (None, None) => String::new(),
    };
    (visible, started.elapsed())
}

async fn deadline_cuts_the_call(lang: Lang) {
    let (visible, took) = run(lang, 600, ExecutionPolicy::Standard, &unique("deadline")).await;
    assert!(
        visible.contains("timed out"),
        "{lang:?}: a call past the deadline must fail as a timeout: {visible}"
    );
    assert!(took < Duration::from_secs(10), "{lang:?}: took {took:?}");
}

async fn a_stop_cuts_the_call(lang: Lang) {
    let id = unique("stop");
    let token = tokio_util::sync::CancellationToken::new();
    let _reg = raisin_storage::jobs::execution_cancel::register(&id, token.clone());
    let t = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        t.cancel();
    });
    let (visible, took) = run(lang, 60_000, ExecutionPolicy::Standard, &id).await;
    assert!(
        visible.contains("cancelled"),
        "{lang:?}: a stopped execution's call must fail as cancelled: {visible}"
    );
    assert!(took < Duration::from_secs(10), "{lang:?}: took {took:?}");
}

async fn a_pure_function_is_refused(lang: Lang) {
    let (visible, _) = run(lang, 5_000, ExecutionPolicy::Deterministic, &unique("pure")).await;
    assert!(
        visible.contains(ExecutionPolicy::HOST_DENIED),
        "{lang:?}: a deterministic function's host call must be refused: {visible}"
    );
}

fn unique(what: &str) -> String {
    format!("contract-{what}-{}", nanoid::nanoid!(8))
}

macro_rules! contract {
    ($lang:ident, $deadline:ident, $stop:ident, $pure:ident $(, #[$attr:meta])?) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        $(#[$attr])?
        async fn $deadline() {
            deadline_cuts_the_call(Lang::$lang).await;
        }
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        $(#[$attr])?
        async fn $stop() {
            a_stop_cuts_the_call(Lang::$lang).await;
        }
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        $(#[$attr])?
        async fn $pure() {
            a_pure_function_is_refused(Lang::$lang).await;
        }
    };
}

contract!(JavaScript, js_deadline, js_stop, js_pure, #[ignore = "lands with the QuickJS step"]);
contract!(Starlark, starlark_deadline, starlark_stop, starlark_pure, #[ignore = "lands with the Starlark step"]);
#[cfg(feature = "wasm")]
mod wasm_contract {
    use super::*;
    contract!(Wasm, wasm_deadline, wasm_stop, wasm_pure, #[ignore = "lands with the WASM step"]);
}
