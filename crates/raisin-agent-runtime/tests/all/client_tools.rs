//! Client tools: a waiting tool's `expires_in_ms` bounds its External request,
//! and `POST .../client-result` (`api_client`) answers only a call the relay
//! parked, only for the run's owner, once.

use raisin_agent_runtime::api::Caller;
use raisin_agent_runtime::api_client::{client_result_on, ClientResultRequest, CLIENT_TOOL_RELAY};
use raisin_agent_runtime::driver::{DriveOutcome, NextAction};
use raisin_agent_runtime::events::{OpOutcome, RunEventKind};
use raisin_agent_runtime::ids::RunId;
use raisin_agent_runtime::lifecycle::OperationSpec;
use raisin_agent_runtime::record::OperationKind;
use raisin_agent_runtime::service_exec::Recovered;
use raisin_agent_runtime::state::{RunOutcome, RunStatus, TerminalStatus};
use raisin_agent_runtime::store::AgentRunStore;
use raisin_agent_runtime::testing::{ExecBehavior, RecordingExecutor};
use serde_json::json;
use std::sync::Arc;

use crate::common::*;

const WAIT_MS: u64 = 5_000;

/// A run (owned by `alice`) whose one tool call parked as `waiting` under
/// `resume_key`, naming `tool`, with an optional deadline.
async fn parked(h: &Harness, tool: &str, resume_key: &str, expires_in_ms: Option<u64>) -> RunId {
    let run = h.create(&format!("/chat/{resume_key}")).await;
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior::default()));
    exec.push(ExecBehavior {
        outcome: Some(OpOutcome::Waiting),
        resume_key: Some(resume_key.into()),
        expires_in_ms,
        ..ExecBehavior::default()
    });
    let spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        input: Some(json!({ "tool": tool, "args": {} })),
        ..OperationSpec::default()
    };
    let done = NextAction::Terminal {
        status: TerminalStatus::Completed,
        outcome: RunOutcome::new("succeeded", None),
    };
    let driver = h.driver(planner(vec![NextAction::Operation(spec), done]), exec);
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Waiting)
    );
    run
}

fn alice() -> Caller {
    Caller {
        id: "alice".into(),
        admin: false,
    }
}

fn body(key: &str, delivery: &str) -> ClientResultRequest {
    ClientResultRequest {
        resume_key: key.into(),
        delivery_id: delivery.into(),
        result: json!({ "ok": true, "content": { "n": 1 }, "untrusted": true }),
        client_tools: Some(json!([{ "name": "x" }])),
        client_context: None,
    }
}

#[tokio::test]
async fn expires_in_ms_sets_the_external_requests_deadline_and_the_sweep_closes_it() {
    let h = Harness::manual();
    let run = parked(&h, CLIENT_TOOL_RELAY, "client:call-1", Some(WAIT_MS)).await;
    let open = h.rec(&run).await.state.open_requests();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].expires_at_ms, Some(T0 + WAIT_MS));
    h.advance(WAIT_MS + 1);
    let recovered = h.svc.recover(&h.scope, "sweeper").await.unwrap();
    assert!(recovered
        .iter()
        .any(|r| matches!(r, Recovered::RequestsExpired { run_id } if *run_id == run)));
    assert!(h.rec(&run).await.state.open_requests().is_empty());
    let closed = h.events(&run).await.into_iter().any(
        |e| matches!(&e.kind, RunEventKind::RequestClosed { reason, .. } if reason == "expired"),
    );
    assert!(
        closed,
        "the model is told through a request_closed(expired) event"
    );
}

#[tokio::test]
async fn a_waiting_tool_without_expires_in_ms_waits_without_a_deadline() {
    let h = Harness::manual();
    let run = parked(&h, "/lib/t", "job-9", None).await;
    assert_eq!(
        h.rec(&run).await.state.open_requests()[0].expires_at_ms,
        None
    );
}

#[tokio::test]
async fn the_owner_answers_a_client_call_once() {
    let h = Harness::manual();
    let run = parked(&h, CLIENT_TOOL_RELAY, "client:call-1", Some(WAIT_MS)).await;
    let first = client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &alice(),
        body("client:call-1", "d1"),
    )
    .await
    .unwrap();
    assert_eq!(first.status, "applied");
    // The same delivery again is a duplicate, not a second resolution.
    let again = client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &alice(),
        body("client:call-1", "d1"),
    )
    .await
    .unwrap();
    assert_eq!(again.status, "duplicate");
    // A new delivery for a request that is no longer open is refused.
    let late = client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &alice(),
        body("client:call-1", "d2"),
    )
    .await
    .unwrap();
    assert_eq!(
        (late.status, late.reason.as_deref()),
        ("rejected", Some("request_not_open"))
    );
    assert_eq!(h.status(&run).await, RunStatus::Queued);
}

#[tokio::test]
async fn the_answer_reaches_the_reducer_as_a_tool_result_envelope() {
    let h = Harness::manual();
    let run = parked(&h, CLIENT_TOOL_RELAY, "client:call-1", Some(WAIT_MS)).await;
    client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &alice(),
        body("client:call-1", "d1"),
    )
    .await
    .unwrap();
    let stored = h
        .store
        .read_result(&h.scope, &run, "ext:d1")
        .await
        .unwrap()
        .expect("the envelope is stored under the control id");
    let env: serde_json::Value = serde_json::from_slice(&stored).unwrap();
    assert_eq!(env["envelope"], "raisin.tool-result/1");
    assert_eq!(env["status"], "succeeded");
    assert_eq!(env["untrusted"], true);
    assert_eq!(env["payload"]["content"], json!({ "n": 1 }));
    assert_eq!(env["client_tools"], json!([{ "name": "x" }]));
}

#[tokio::test]
async fn someone_else_is_refused() {
    let h = Harness::manual();
    let run = parked(&h, CLIENT_TOOL_RELAY, "client:call-1", Some(WAIT_MS)).await;
    let mallory = Caller {
        id: "mallory".into(),
        admin: false,
    };
    let r = client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &mallory,
        body("client:call-1", "d1"),
    )
    .await
    .unwrap();
    assert_eq!(
        (r.status, r.reason.as_deref()),
        ("rejected", Some("forbidden"))
    );
    assert_eq!(
        h.rec(&run).await.state.open_requests().len(),
        1,
        "still open"
    );
}

#[tokio::test]
async fn another_tools_external_wait_is_not_a_client_call() {
    let h = Harness::manual();
    // Parked by some other tool, but under a key that LOOKS like a client call.
    let run = parked(&h, "/lib/some-tool", "client:call-1", None).await;
    let r = client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &alice(),
        body("client:call-1", "d1"),
    )
    .await
    .unwrap();
    assert_eq!(
        (r.status, r.reason.as_deref()),
        ("rejected", Some("not_a_client_call"))
    );
    // And a key without the prefix never is, open or not.
    let run2 = parked(&h, "/lib/some-tool", "job-7", None).await;
    let r2 = client_result_on(&h.svc, &h.scope, &run2, &alice(), body("job-7", "d2"))
        .await
        .unwrap();
    assert_eq!(
        (r2.status, r2.reason.as_deref()),
        ("rejected", Some("not_a_client_call"))
    );
    assert_eq!(
        h.rec(&run2).await.state.open_requests().len(),
        1,
        "untouched"
    );
}

#[tokio::test]
async fn an_answer_after_the_deadline_is_refused_as_expired() {
    let h = Harness::manual();
    let run = parked(&h, CLIENT_TOOL_RELAY, "client:call-1", Some(WAIT_MS)).await;
    h.advance(WAIT_MS + 1);
    let r = client_result_on(
        &h.svc,
        &h.scope,
        &run,
        &alice(),
        body("client:call-1", "d1"),
    )
    .await
    .unwrap();
    assert_eq!(
        (r.status, r.reason.as_deref()),
        ("rejected", Some("expired"))
    );
}

#[tokio::test]
async fn an_unknown_run_is_not_found() {
    let h = Harness::manual();
    let err = client_result_on(
        &h.svc,
        &h.scope,
        &RunId("nope".into()),
        &alice(),
        body("client:c", "d"),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status, 404);
}
