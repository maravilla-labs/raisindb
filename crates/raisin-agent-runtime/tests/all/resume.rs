//! Crash, recovery, pause/resume, budgets, request expiry, external results.

use raisin_agent_contract::{
    effect_id, Effect, EffectBody, EventKind, ReducerResponse, CONTRACT_V1,
};
use raisin_agent_runtime::control::{ControlAck, ControlKind};
use raisin_agent_runtime::domain::ReducerRef;
use raisin_agent_runtime::driver::{DriveOutcome, Mode, NextAction, RunDriver};
use raisin_agent_runtime::events::{CheckpointReason, OpOutcome, RunEventKind};
use raisin_agent_runtime::ids::SystemToken;
use raisin_agent_runtime::lifecycle::{NewRequest, OperationSpec};
use raisin_agent_runtime::record::{OperationKind, PendingKind, RunBudgets};
use raisin_agent_runtime::service_exec::Recovered;
use raisin_agent_runtime::state::{
    PauseReason, RunOutcome, RunState, RunStatus, TerminalStatus, WakeReason,
};
use raisin_agent_runtime::store::AgentRunStore;
use raisin_agent_runtime::testing::{ExecBehavior, RecordingExecutor, ScriptedReducer};
use serde_json::json;
use std::sync::Arc;

use crate::common::*;
use crate::lease::crashing_driver;

fn done() -> NextAction {
    NextAction::Terminal {
        status: TerminalStatus::Completed,
        outcome: RunOutcome::new("succeeded", None),
    }
}

#[tokio::test]
async fn driver_crash_mid_op_then_recover_resumes_to_completion() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let exec = executor(0);
    let d1 = crashing_driver(
        &h,
        vec![op_with(|s| s.replay_safe = Some(true)), done()],
        exec.clone(),
    );
    assert_eq!(
        d1.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Crashed
    );
    h.advance(TTL + 1);
    // The recovering driver has the rest of the plan.
    let d2 = h.driver(planner(vec![done()]), exec.clone());
    let out = d2.recover_and_drive(&h.scope, "w2").await.unwrap();
    assert_eq!(
        out,
        vec![(run.clone(), DriveOutcome::Exited(RunStatus::Completed))]
    );
    assert_eq!(
        exec.total_side_effects(),
        1,
        "every side effect counted once"
    );
    assert_clean_terminal(&h.rec(&run).await);
}

#[tokio::test]
async fn crash_during_cancelling_resolves_to_stopped_on_recovery() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let g = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    h.svc
        .begin_operation(&h.scope, &run, &g.fence, OperationSpec::default())
        .await
        .unwrap();
    h.svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Cancelling);
    h.advance(TTL + 1); // the driver died without acknowledging
    let rec = h.svc.recover(&h.scope, "w2").await.unwrap();
    assert!(
        rec.contains(&Recovered::TakenOver {
            run_id: run.clone(),
            status: RunStatus::Stopped
        }),
        "{rec:?}"
    );
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationCancelled {
                acknowledged: false,
                ..
            }
        )),
        1
    );
    assert_clean_terminal(&h.rec(&run).await);
}

#[tokio::test]
async fn pause_writes_checkpoint_and_resume_continues_from_it() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    h.svc
        .submit_control(&h.scope, &run, cmd("p1", ControlKind::Pause), None)
        .await
        .unwrap();
    let rec = h.rec(&run).await;
    assert!(matches!(
        rec.state,
        RunState::Paused {
            reason: PauseReason::User,
            ..
        }
    ));
    let ckpt = h
        .store
        .latest_checkpoint(&h.scope, &run)
        .await
        .unwrap()
        .expect("checkpoint");
    assert_eq!(ckpt.reason, CheckpointReason::Pause);
    assert_eq!(ckpt.core.status, RunStatus::Paused);
    assert_eq!(rec.last_checkpoint_seq, Some(ckpt.at_seq));
    let resume = cmd(
        "r1",
        ControlKind::Resume {
            budget_increase: None,
            accept_reducer_change: false,
        },
    );
    h.svc
        .submit_control(&h.scope, &run, resume, None)
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Queued);
    let events = h.events(&run).await;
    assert!(events.iter().any(|e| matches!(
        e.kind,
        RunEventKind::Resumed {
            from_checkpoint_no: Some(1)
        }
    )));
    let exec = executor(0);
    let out = h
        .driver(planner(vec![tool_op(), done()]), exec.clone())
        .drive(&h.scope, &run, "w2")
        .await
        .unwrap();
    assert_eq!(out, DriveOutcome::Exited(RunStatus::Completed));
    assert_eq!(exec.total_side_effects(), 1);
}

/// run_started → call_tool; tool_result → complete; anything else: no effects.
pub fn tool_then_complete(hash: &str) -> Arc<ScriptedReducer> {
    ScriptedReducer::new(hash, |req| {
        let (rev, effects) = match req.event.kind {
            EventKind::RunStarted => {
                let rev = req.state_rev + 1;
                let body = EffectBody::CallTool {
                    tool: "/lib/t".into(),
                    args: json!({ "a": 1 }),
                    mutating: false,
                    replay_safe: true,
                    interruptible: true,
                    timeout_ms: None,
                    for_call_id: None,
                };
                (
                    rev,
                    vec![Effect {
                        effect_id: effect_id(rev, 0),
                        body,
                    }],
                )
            }
            EventKind::ToolResult => {
                let rev = req.state_rev + 1;
                let body = EffectBody::Complete {
                    outcome: raisin_agent_contract::CompleteOutcome::Succeeded,
                    summary: None,
                    artifacts: vec![],
                    evidence: vec![],
                };
                (
                    rev,
                    vec![Effect {
                        effect_id: effect_id(rev, 0),
                        body,
                    }],
                )
            }
            _ => (req.state_rev, vec![]),
        };
        let state = if rev == req.state_rev {
            req.state.clone().unwrap_or(json!({}))
        } else {
            json!({ "last_event_seq": req.event.seq })
        };
        Ok(ReducerResponse {
            contract: CONTRACT_V1.into(),
            state,
            state_rev: rev,
            effects,
            projection: None,
            diagnostics: vec![],
            refused: None,
        })
    })
}

pub fn reducer_ref(hash: &str) -> ReducerRef {
    ReducerRef {
        function_path: "/lib/test/reducer".into(),
        handler: "reduce".into(),
        artifact_hash: hash.into(),
    }
}

#[tokio::test]
async fn budget_exceeded_pauses_keeps_outbox_then_resume_with_increase_dispatches_it() {
    let h = Harness::manual();
    let mut req = h.request("/a");
    req.reducer = Some(reducer_ref("h1"));
    req.budgets = RunBudgets {
        max_operations: Some(0),
        ..RunBudgets::default()
    };
    let run = h.create_with(req).await;
    let reducer = tool_then_complete("h1");
    let exec = executor(0);
    let driver = RunDriver::new(h.svc.clone(), Mode::Domain(reducer.clone()), exec.clone());
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Paused)
    );
    let rec = h.rec(&run).await;
    assert!(matches!(
        rec.state,
        RunState::Paused {
            reason: PauseReason::Budget { .. },
            ..
        }
    ));
    assert!(
        rec.domain.as_ref().unwrap().outbox.is_some(),
        "the effect waits in the outbox"
    );
    assert_eq!(exec.total_side_effects(), 0);
    let inc = RunBudgets {
        max_operations: Some(5),
        ..RunBudgets::default()
    };
    h.svc
        .submit_control(
            &h.scope,
            &run,
            cmd(
                "r",
                ControlKind::Resume {
                    budget_increase: Some(inc),
                    accept_reducer_change: false,
                },
            ),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        driver.drive(&h.scope, &run, "w2").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Completed)
    );
    assert_eq!(exec.total_side_effects(), 1);
    assert_eq!(
        reducer
            .calls()
            .iter()
            .filter(|r| r.event.kind == EventKind::RunStarted)
            .count(),
        1,
        "the reducer was not re-asked"
    );
    let rec = h.rec(&run).await;
    assert_clean_terminal(&rec);
    assert!(rec.domain.as_ref().unwrap().finalized);
}

#[tokio::test]
async fn pending_request_expires() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let wait = NextAction::Wait(vec![NewRequest {
        kind: PendingKind::Input {
            prompt: "?".into(),
            choices: None,
            schema: None,
        },
        effect_id: None,
        expires_at_ms: Some(T0 + 1_000),
    }]);
    h.driver(planner(vec![wait]), executor(0))
        .drive(&h.scope, &run, "w1")
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Waiting);
    h.waker.drain();
    h.advance(2_000);
    let rec = h.svc.recover(&h.scope, "sweeper").await.unwrap();
    assert!(rec.contains(&Recovered::RequestsExpired {
        run_id: run.clone()
    }));
    assert_eq!(h.status(&run).await, RunStatus::Queued);
    assert!(h
        .waker
        .calls()
        .contains(&(run.clone(), WakeReason::RequestResolved)));
    let events = h.events(&run).await;
    assert_eq!(
        count(
            &events,
            |k| matches!(k, RunEventKind::RequestClosed { reason, .. } if reason == "expired")
        ),
        1
    );
}

#[tokio::test]
async fn waiting_tool_outcome_opens_external_request_and_delivery_resumes() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior::default()));
    exec.push(ExecBehavior {
        outcome: Some(OpOutcome::Waiting),
        resume_key: Some("job-7".into()),
        ..ExecBehavior::default()
    });
    let spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        ..OperationSpec::default()
    };
    let driver = h.driver(
        planner(vec![NextAction::Operation(spec), done()]),
        exec.clone(),
    );
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Waiting)
    );
    let open = h.rec(&run).await.state.open_requests();
    assert!(
        matches!(&open[0].kind, PendingKind::External { resume_key, .. } if resume_key == "job-7")
    );
    let token = SystemToken::in_process();
    let envelope =
        json!({ "envelope": "raisin.tool-result/1", "operation_id": "x", "status": "succeeded" });
    let ack = h
        .svc
        .deliver_external_result(&h.scope, &run, "job-7", "d1", envelope.clone(), &token)
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Applied { .. }));
    assert_eq!(h.status(&run).await, RunStatus::Queued);
    assert!(h
        .waker
        .calls()
        .contains(&(run.clone(), WakeReason::ExternalResult)));
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Completed)
    );
}

#[tokio::test]
async fn external_result_delivery_is_deduped() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior {
        outcome: Some(OpOutcome::Waiting),
        resume_key: Some("k".into()),
        ..ExecBehavior::default()
    }));
    h.driver(planner(vec![tool_op()]), exec)
        .drive(&h.scope, &run, "w1")
        .await
        .unwrap();
    let token = SystemToken::in_process();
    let first = h
        .svc
        .deliver_external_result(&h.scope, &run, "k", "d1", json!({ "x": 1 }), &token)
        .await
        .unwrap();
    let n = h.events(&run).await.len();
    let again = h
        .svc
        .deliver_external_result(&h.scope, &run, "k", "d1", json!({ "x": 1 }), &token)
        .await
        .unwrap();
    assert_eq!(
        again,
        ControlAck::Duplicate {
            original_seq: first.seq()
        }
    );
    assert_eq!(h.events(&run).await.len(), n);
}

/// An externally delivered result reaches the reducer shaped like a direct
/// completion: the tool (and the model call it answers) are named, and the
/// envelope is the delivered one.
#[tokio::test]
async fn external_result_reaches_the_reducer_as_a_named_tool_result() {
    let h = Harness::manual();
    let mut req = h.request("/a");
    req.reducer = Some(reducer_ref("h1"));
    let run = h.create_with(req).await;
    let reducer = tool_then_complete("h1");
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior {
        outcome: Some(OpOutcome::Waiting),
        resume_key: Some("job-9".into()),
        ..ExecBehavior::default()
    }));
    let driver = RunDriver::new(h.svc.clone(), Mode::Domain(reducer.clone()), exec);
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Waiting)
    );
    let envelope =
        json!({ "envelope": "raisin.tool-result/1", "operation_id": "x", "status": "succeeded" });
    h.svc
        .deliver_external_result(
            &h.scope,
            &run,
            "job-9",
            "d9",
            envelope.clone(),
            &SystemToken::in_process(),
        )
        .await
        .unwrap();
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Completed)
    );
    let calls = reducer.calls();
    let delivered = calls
        .iter()
        .find(|c| c.event.kind == EventKind::ToolResult)
        .expect("the external result was delivered");
    assert_eq!(delivered.event.data["tool"], json!("/lib/t"));
    assert_eq!(delivered.event.data["envelope"], envelope);
    assert_eq!(delivered.event.effect_id.as_deref(), Some("1:0"));
}
