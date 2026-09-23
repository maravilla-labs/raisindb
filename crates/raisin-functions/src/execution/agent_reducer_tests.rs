// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! One reducer contract, several runtimes: the same `FunctionDomainReducer`
//! drives a JavaScript reducer (QuickJS) and a WebAssembly component, both
//! under the generic deterministic policy.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_agent_runtime::contract::{
    EffectBody, EventKind, ReducerEvent, ReducerRequest, RunStatus, RunView, CONTRACT_V1,
};
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use serde_json::{json, Value};

use super::*;
use crate::runtime::QuickJsRuntime;

pub(crate) fn request(data: Value) -> ReducerRequest {
    ReducerRequest {
        contract: CONTRACT_V1.into(),
        accept: vec![CONTRACT_V1.into()],
        run: RunView {
            run_id: "run-1".into(),
            status: RunStatus::Running,
            turn: Some(1),
            last_seq: 3,
            usage: json!({}),
            budgets: json!({}),
            open_requests: vec![],
            unanswered_calls: vec![],
            scope: None,
            subject: None,
        },
        state: None,
        state_rev: 0,
        event: ReducerEvent {
            seq: 1,
            kind: EventKind::RunStarted,
            effect_id: None,
            operation_id: None,
            data,
        },
    }
}

/// A reducer in plain JavaScript. `mode` selects the behaviour under test.
const JS_REDUCER: &str = r#"
async function handler(req) {
  const mode = (req.event.data && req.event.data.mode) || "ok";
  const rev = req.state_rev + 1;
  const tool = (id) => ({ effect_id: `${rev}:${id}`, kind: "call_tool", tool: "/lib/demo/read",
                          args: {}, mutating: false, replay_safe: true });
  if (mode === "ok") {
    return { contract: req.contract, state: { last_event_seq: req.event.seq }, state_rev: rev,
             effects: [tool(0)] };
  }
  if (mode === "two_ops") {
    return { contract: req.contract, state: {}, state_rev: rev, effects: [tool(0), tool(1)] };
  }
  if (mode === "call_host") {
    try {
      await raisin.nodes.create("default", "/", { name: "x", node_type: "raisin:Folder" });
      return { contract: req.contract, state: {}, state_rev: req.state_rev, effects: [] };
    } catch (e) {
      return { contract: req.contract, state: {}, state_rev: req.state_rev, effects: [],
               refused: { code: "host_denied", message: String(e && e.message || e) } };
    }
  }
  if (mode === "ambient") {
    return { contract: req.contract, state_rev: rev, effects: [],
             state: { now: Date.now(), date: new Date().getTime(), r: [Math.random(), Math.random()] } };
  }
  if (mode === "throw") { throw new Error("reducer bug"); }
  return null;
}
"#;

fn js_invoker(source: &str) -> Arc<dyn DeterministicInvoker> {
    Arc::new(DirectRuntimeInvoker::new(
        Arc::new(QuickJsRuntime::new()),
        FunctionMetadata::javascript("js-reducer").with_entry_file("index.js:handler"),
        FunctionCode::Text(source.to_string()),
        HashMap::new(),
    ))
}

async fn js_reducer() -> FunctionDomainReducer {
    FunctionDomainReducer::bind("/lib/test/js-reducer", "handler", js_invoker(JS_REDUCER))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_roundtrip_valid_response() {
    let resp = js_reducer()
        .await
        .reduce(&request(json!({})))
        .await
        .unwrap();
    assert_eq!(resp.state_rev, 1);
    assert_eq!(resp.effects.len(), 1);
    assert_eq!(resp.effects[0].effect_id, "1:0");
    assert!(matches!(resp.effects[0].body, EffectBody::CallTool { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_invalid_response_refused_r4() {
    let err = js_reducer()
        .await
        .reduce(&request(json!({ "mode": "two_ops" })))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReducerCallError::Invalid(ref r) if r.code == "multiple_operations"),
        "{err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_host_call_denied_by_policy() {
    let err = js_reducer()
        .await
        .reduce(&request(json!({ "mode": "call_host" })))
        .await
        .unwrap_err();
    match err {
        ReducerCallError::Refused { code, message } => {
            assert_eq!(code, "host_denied");
            assert!(message.contains("denied"), "{message}");
        }
        other => panic!("expected a denied host call, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_sees_no_ambient_time_or_entropy() {
    let r = js_reducer().await;
    let a = r
        .reduce(&request(json!({ "mode": "ambient" })))
        .await
        .unwrap();
    let b = r
        .reduce(&request(json!({ "mode": "ambient" })))
        .await
        .unwrap();
    assert_eq!(a.state["now"], json!(0), "Date.now is frozen");
    assert_eq!(a.state["date"], json!(0), "new Date() is the epoch");
    assert_eq!(a, b, "identical input, identical output");
}

#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_throw_is_a_refusal() {
    let err = js_reducer()
        .await
        .reduce(&request(json!({ "mode": "throw" })))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReducerCallError::Refused { ref code, .. } if code == "handler_error"),
        "{err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_changed_source_is_detected() {
    let pinned = js_reducer().await.reducer_ref().clone();
    let edited = format!("{JS_REDUCER}\n// edited");
    let err = FunctionDomainReducer::new(pinned, js_invoker(&edited))
        .reduce(&request(json!({})))
        .await
        .unwrap_err();
    assert!(matches!(err, ReducerCallError::Changed { .. }), "{err:?}");
}

/// The JavaScript reducer plugged into the real runtime: a domain run driven
/// by it until its budget pauses it — no WebAssembly anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn quickjs_reducer_drives_a_run() {
    use raisin_agent_runtime::conformance::{create_request, scope};
    use raisin_agent_runtime::driver::{DriveOutcome, Mode, RunDriver};
    use raisin_agent_runtime::memory::InMemoryAgentRunStore;
    use raisin_agent_runtime::service::{AgentRunService, ServiceConfig};
    use raisin_agent_runtime::store::CreateOutcome;
    use raisin_agent_runtime::testing::{ExecBehavior, RecordingExecutor};

    let r = Arc::new(js_reducer().await);
    let svc = Arc::new(AgentRunService::new(
        Arc::new(InMemoryAgentRunStore::default()),
        Arc::new(raisin_agent_runtime::wake::NoopWaker),
        Arc::new(raisin_agent_runtime::clock::ManualClock::new(1_000)),
        ServiceConfig::default(),
    ));
    let s = scope("t");
    let mut req = create_request(&s, "/chat");
    req.reducer = Some(r.reducer_ref().clone());
    req.budgets.max_operations = Some(1);
    let CreateOutcome::Created { run_id, .. } = svc.create(req, None).await.unwrap() else {
        panic!("created");
    };
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior {
        payload: Some(
            json!({ "envelope": "raisin.tool-result/1", "operation_id": "x", "status": "succeeded" }),
        ),
        ..ExecBehavior::default()
    }));
    let out = RunDriver::new(svc.clone(), Mode::Domain(r), exec.clone())
        .drive(&s, &run_id, "w")
        .await
        .unwrap();
    assert_eq!(out, DriveOutcome::Exited(RunStatus::Paused));
    assert_eq!(exec.total_side_effects(), 1);
}

/// Kept here so the ref type stays in use when the wasm feature is off.
#[allow(dead_code)]
fn _ref(r: &ReducerRef) -> &str {
    &r.function_path
}

#[cfg(feature = "wasm")]
#[path = "agent_reducer_wasm_tests.rs"]
mod wasm;
