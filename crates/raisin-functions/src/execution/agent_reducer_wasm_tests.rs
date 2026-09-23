// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The SAME `FunctionDomainReducer`, over a real WebAssembly component
//! (`fixtures/wasm-guests/agent-reducer`, committed as `agent_reducer.wasm`).
//! WebAssembly's frozen clocks and fixed entropy are just its implementation
//! of the generic deterministic policy.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_agent_runtime::contract::EffectBody;
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use serde_json::json;

use super::super::*;
use super::request;
use crate::runtime::WasmRuntime;

const AGENT_REDUCER: &[u8] = include_bytes!("../runtime/wasm/fixtures/agent_reducer.wasm");

fn invoker(handler: &str) -> Arc<dyn DeterministicInvoker> {
    Arc::new(DirectRuntimeInvoker::new(
        Arc::new(WasmRuntime::new()),
        FunctionMetadata::wasm("agent-reducer")
            .with_entry_file(format!("agent_reducer.wasm:{handler}")),
        FunctionCode::Bytes(Arc::from(AGENT_REDUCER.to_vec())),
        HashMap::new(),
    ))
}

async fn reducer() -> FunctionDomainReducer {
    FunctionDomainReducer::bind("/lib/test/agent-reducer", "reduce", invoker("reduce"))
        .await
        .unwrap()
}

#[tokio::test]
async fn wasm_reducer_roundtrip_valid_response() {
    let resp = reducer().await.reduce(&request(json!({}))).await.unwrap();
    assert_eq!(resp.state_rev, 1);
    assert_eq!(resp.effects.len(), 1);
    assert_eq!(resp.effects[0].effect_id, "1:0");
    assert!(matches!(resp.effects[0].body, EffectBody::CallTool { .. }));
}

#[tokio::test]
async fn wasm_reducer_invalid_response_refused_r4() {
    let err = reducer()
        .await
        .reduce(&request(json!({ "mode": "two_ops" })))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReducerCallError::Invalid(ref r) if r.code == "multiple_operations"),
        "{err:?}"
    );
}

#[tokio::test]
async fn wasm_reducer_host_call_denied() {
    let err = reducer()
        .await
        .reduce(&request(json!({ "mode": "call_host" })))
        .await
        .unwrap_err();
    match err {
        ReducerCallError::Refused { code, message } => {
            assert_eq!(code, "host_denied");
            assert!(message.contains("denied"), "{message}");
        }
        other => panic!("expected the guest to report a denied host call, got {other:?}"),
    }
}

#[tokio::test]
async fn wasm_reducer_sees_no_ambient_time_or_entropy() {
    let r = reducer().await;
    let a = r
        .reduce(&request(json!({ "mode": "ambient" })))
        .await
        .unwrap();
    let b = r
        .reduce(&request(json!({ "mode": "ambient" })))
        .await
        .unwrap();
    assert_eq!(a.state["wall_ns"], json!(0), "wall clock is frozen");
    assert_eq!(a.state["mono_ns"], json!(0), "monotonic clock is frozen");
    assert_eq!(a, b, "identical input, identical output");
}

#[tokio::test]
async fn wasm_reducer_hash_mismatch_detected() {
    let stale = ReducerRef {
        function_path: "/lib/test/agent-reducer".into(),
        handler: "reduce".into(),
        artifact_hash: "0".repeat(64),
    };
    let err = FunctionDomainReducer::new(stale, invoker("reduce"))
        .reduce(&request(json!({})))
        .await
        .unwrap_err();
    assert!(matches!(err, ReducerCallError::Changed { .. }), "{err:?}");
}

#[tokio::test]
async fn wasm_reducer_trap_is_unavailable_not_refused() {
    let err = reducer()
        .await
        .reduce(&request(json!({ "mode": "trap" })))
        .await
        .unwrap_err();
    assert!(matches!(err, ReducerCallError::Unavailable(_)), "{err:?}");
}

#[tokio::test]
async fn wasm_reducer_unknown_handler_is_a_refusal() {
    let r = FunctionDomainReducer::bind("/lib/test/agent-reducer", "nope", invoker("nope"))
        .await
        .unwrap();
    let err = r.reduce(&request(json!({}))).await.unwrap_err();
    assert!(
        matches!(err, ReducerCallError::Refused { ref code, .. } if code == "handler_error"),
        "{err:?}"
    );
}
