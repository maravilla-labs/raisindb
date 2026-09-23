//! Fencing: a stale driver can never write.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_contract::{
    effect_id, Effect, EffectBody, ReducerRequest, ReducerResponse, CONTRACT_V1,
};
use raisin_agent_runtime::control::ControlKind;
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use raisin_agent_runtime::driver::{DriveOutcome, Mode, RunDriver};
use raisin_agent_runtime::events::{OpOutcome, RunEventKind};
use raisin_agent_runtime::ids::{RunId, RunScope};
use raisin_agent_runtime::lifecycle::{
    self, apply_control, OperationResult, OperationSpec, TRANSITIONS,
};
use raisin_agent_runtime::record::{AgentRunRecord, OperationKind};
use raisin_agent_runtime::service::{AgentRunService, ServiceError};
use raisin_agent_runtime::service_exec::Recovered;
use raisin_agent_runtime::state::{Activity, RunOutcome, RunState, RunStatus, TerminalStatus};
use raisin_agent_runtime::store::StoreError;
use serde_json::json;

use crate::common::*;

/// Submits a stop from INSIDE its reducer call, then answers with a tool call.
struct StoppingReducer {
    r: ReducerRef,
    svc: Arc<AgentRunService>,
    scope: RunScope,
    run: std::sync::Mutex<Option<RunId>>,
}

#[async_trait]
impl DomainReducer for StoppingReducer {
    fn reducer_ref(&self) -> &ReducerRef {
        &self.r
    }
    async fn reduce(&self, req: &ReducerRequest) -> Result<ReducerResponse, ReducerCallError> {
        let run = self.run.lock().unwrap().clone().unwrap();
        self.svc
            .submit_control(&self.scope, &run, stop("c-mid"), None)
            .await
            .unwrap();
        let rev = req.state_rev + 1;
        Ok(ReducerResponse {
            contract: CONTRACT_V1.into(),
            state: json!({ "n": 1 }),
            state_rev: rev,
            effects: vec![Effect {
                effect_id: effect_id(rev, 0),
                body: EffectBody::CallTool {
                    tool: "/lib/t".into(),
                    args: json!({}),
                    mutating: true,
                    replay_safe: false,
                    interruptible: true,
                    timeout_ms: None,
                    for_call_id: None,
                },
            }],
            projection: None,
            diagnostics: vec![],
            refused: None,
        })
    }
}

#[tokio::test]
async fn driver_commit_after_stop_between_ops_is_refused() {
    let h = Harness::manual();
    let r = ReducerRef {
        function_path: "/r".into(),
        handler: "reduce".into(),
        artifact_hash: "h".into(),
    };
    let mut req = h.request("/a");
    req.reducer = Some(r.clone());
    let run = h.create_with(req).await;
    let reducer = Arc::new(StoppingReducer {
        r,
        svc: h.svc.clone(),
        scope: h.scope.clone(),
        run: std::sync::Mutex::new(Some(run.clone())),
    });
    let exec = executor(0);
    let driver = RunDriver::new(h.svc.clone(), Mode::Domain(reducer), exec.clone());
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::LeaseLost
    );
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(k, RunEventKind::DomainApplied { .. })),
        0,
        "the driver wrote nothing"
    );
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationStarted { .. }
        )),
        0
    );
    assert_eq!(exec.total_side_effects(), 0);
    assert_eq!(h.status(&run).await, RunStatus::Stopped);
}

async fn running_op(
    h: &Harness,
    replay_safe: bool,
) -> (
    RunId,
    lifecycle::LeaseFence,
    raisin_agent_runtime::record::ActiveOperation,
) {
    let run = h.create("/a").await;
    let grant = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    let spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        replay_safe: Some(replay_safe),
        ..OperationSpec::default()
    };
    let ticket = h
        .svc
        .begin_operation(&h.scope, &run, &grant.fence, spec)
        .await
        .unwrap();
    (run, grant.fence, ticket.op)
}

#[tokio::test]
async fn stale_worker_commit_after_takeover_is_rejected_lease_lost() {
    let h = Harness::manual();
    let (run, fence, op) = running_op(&h, true).await;
    h.advance(TTL + 1);
    let rec = h.svc.recover(&h.scope, "w2").await.unwrap();
    assert!(
        matches!(&rec[..], [Recovered::Redispatch { .. }, ..]),
        "{rec:?}"
    );
    let err = h
        .svc
        .finish_operation(
            &h.scope,
            &run,
            &fence,
            &op.op_id,
            OperationResult::default(),
        )
        .await
        .unwrap_err();
    assert!(err.is_lease_lost(), "{err:?}");
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationCompleted { .. }
        )),
        0
    );
}

#[tokio::test]
async fn commit_after_lease_expiry_without_takeover_is_refused() {
    let h = Harness::manual();
    let (run, fence, _) = running_op(&h, true).await;
    h.advance(TTL);
    let err = h.svc.renew_lease(&h.scope, &run, &fence).await.unwrap_err();
    assert!(matches!(
        err,
        ServiceError::Store(StoreError::LeaseLost { .. })
    ));
}

#[tokio::test]
async fn renew_extends_expiry_and_fails_after_takeover() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let grant = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    h.advance(60_000);
    let rec = h
        .svc
        .renew_lease(&h.scope, &run, &grant.fence)
        .await
        .unwrap();
    assert_eq!(rec.state.lease().unwrap().expires_at_ms, T0 + 60_000 + TTL);
    h.advance(60_000); // past the ORIGINAL expiry, within the renewed one
    h.svc
        .renew_lease(&h.scope, &run, &grant.fence)
        .await
        .unwrap();
    h.advance(TTL + 1);
    h.svc.recover(&h.scope, "w2").await.unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Queued);
    assert!(h
        .svc
        .renew_lease(&h.scope, &run, &grant.fence)
        .await
        .unwrap_err()
        .is_lease_lost());
}

fn idle(h_rec: &AgentRunRecord) -> bool {
    matches!(
        h_rec.state,
        RunState::Running {
            activity: Activity::Idle { .. },
            ..
        }
    )
}

/// Every transition that clears a lease bumps `lease_epoch`, and every status
/// change it makes is in the transition table.
#[tokio::test]
async fn every_lease_clearing_transition_bumps_epoch() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let queued = h.rec(&run).await;
    let now = T0;
    let idle_rec = lifecycle::apply_acquire(&queued, "w1", now, TTL)
        .unwrap()
        .record;
    assert!(idle(&idle_rec));
    let op_spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        ..OperationSpec::default()
    };
    let operating = lifecycle::apply_begin(&idle_rec, op_spec.clone(), now)
        .unwrap()
        .record;
    let op_id = operating.state.active_op().unwrap().op_id.clone();
    let cancelling = apply_control(&operating, &stop("s"), true, now)
        .transition
        .record;
    let pause_requested = apply_control(&operating, &cmd("p", ControlKind::Pause), true, now)
        .transition
        .record;
    let safe_operating = lifecycle::apply_begin(
        &idle_rec,
        OperationSpec {
            replay_safe: Some(true),
            ..op_spec
        },
        now,
    )
    .unwrap()
    .record;
    let exceeded = raisin_agent_runtime::budget::Exceeded {
        which: "max_operations".into(),
        limit: 0,
        used: 0,
    };
    let mut failing = idle_rec.clone();
    failing.budgets.on_exceeded = raisin_agent_runtime::record::BudgetPolicy::Fail;
    let waiting_result = OperationResult {
        outcome: Some(OpOutcome::Waiting),
        resume_key: Some("k".into()),
        ..OperationResult::default()
    };
    let cases: Vec<(&str, AgentRunRecord, AgentRunRecord)> = vec![
        (
            "stop at idle",
            idle_rec.clone(),
            apply_control(&idle_rec, &stop("s"), true, now)
                .transition
                .record,
        ),
        (
            "pause at idle",
            idle_rec.clone(),
            apply_control(&idle_rec, &cmd("p", ControlKind::Pause), true, now)
                .transition
                .record,
        ),
        (
            "finish with pause requested",
            pause_requested.clone(),
            lifecycle::apply_finish(&pause_requested, &op_id, OperationResult::default(), now)
                .unwrap()
                .record,
        ),
        (
            "finish waiting",
            operating.clone(),
            lifecycle::apply_finish(&operating, &op_id, waiting_result, now)
                .unwrap()
                .record,
        ),
        (
            "finish while cancelling",
            cancelling.clone(),
            lifecycle::apply_finish(&cancelling, &op_id, OperationResult::default(), now)
                .unwrap()
                .record,
        ),
        (
            "terminal at idle",
            idle_rec.clone(),
            lifecycle::apply_terminal(
                &idle_rec,
                TerminalStatus::Completed,
                RunOutcome::default(),
                None,
                now,
            )
            .unwrap()
            .record,
        ),
        (
            "release",
            idle_rec.clone(),
            lifecycle::apply_release(&idle_rec, now, true)
                .unwrap()
                .record,
        ),
        (
            "budget pause",
            idle_rec.clone(),
            lifecycle::apply_budget_exceeded(&idle_rec, &exceeded, now).record,
        ),
        (
            "budget fail",
            failing.clone(),
            lifecycle::apply_budget_exceeded(&failing, &exceeded, now).record,
        ),
        (
            "reducer pause",
            idle_rec.clone(),
            lifecycle::apply_reducer_pause(&idle_rec, "reducer_unavailable", now)
                .unwrap()
                .record,
        ),
        (
            "takeover idle",
            idle_rec.clone(),
            lifecycle::apply_takeover(&idle_rec, "w2", now, TTL)
                .unwrap()
                .transition
                .record,
        ),
        (
            "takeover abandon",
            operating.clone(),
            lifecycle::apply_takeover(&operating, "w2", now, TTL)
                .unwrap()
                .transition
                .record,
        ),
        (
            "takeover cancelling",
            cancelling.clone(),
            lifecycle::apply_takeover(&cancelling, "w2", now, TTL)
                .unwrap()
                .transition
                .record,
        ),
        (
            "takeover redispatch",
            safe_operating.clone(),
            lifecycle::apply_takeover(&safe_operating, "w2", now, TTL)
                .unwrap()
                .transition
                .record,
        ),
    ];
    for (name, before, after) in cases {
        assert!(
            after.lease_epoch > before.lease_epoch,
            "{name}: epoch {} -> {}",
            before.lease_epoch,
            after.lease_epoch
        );
        if let Some(lease) = after.state.lease() {
            assert_eq!(
                lease.epoch, after.lease_epoch,
                "{name}: a new lease carries the new epoch"
            );
        }
        let (from, to) = (before.state.status(), after.state.status());
        if from != to {
            assert!(
                TRANSITIONS.iter().any(|(f, t, _)| *f == from && *t == to),
                "{name}: {from:?} -> {to:?} is not in the table"
            );
        }
    }
}
