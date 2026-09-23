//! Leases: expiry, takeover, replay-safe re-dispatch, abandonment.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_contract::EventKind;
use raisin_agent_runtime::domain::{map_event, ResultLoader};
use raisin_agent_runtime::driver::{DriveOutcome, DriverHooks, HookAction, Mode, RunDriver};
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::lifecycle::{OperationResult, OperationSpec};
use raisin_agent_runtime::record::{ActiveOperation, OperationKind};
use raisin_agent_runtime::service_exec::Recovered;
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::testing::RecordingExecutor;
use serde_json::Value;

use crate::common::*;

/// Crashes the first time an operation returns, before `finish_operation`.
#[derive(Default)]
pub struct CrashOnce(AtomicBool);

#[async_trait]
impl DriverHooks for CrashOnce {
    async fn after_execute(&self, _op: &ActiveOperation, _r: &OperationResult) -> HookAction {
        if self.0.swap(true, Ordering::SeqCst) {
            HookAction::Continue
        } else {
            HookAction::Crash
        }
    }
}

pub fn crashing_driver(
    h: &Harness,
    actions: Vec<raisin_agent_runtime::driver::NextAction>,
    exec: Arc<RecordingExecutor>,
) -> RunDriver {
    RunDriver::new(h.svc.clone(), Mode::Planner(planner(actions)), exec)
        .with_hooks(Arc::new(CrashOnce::default()))
}

#[tokio::test]
async fn lease_expiry_allows_takeover_with_higher_epoch() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let g1 = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    assert!(
        h.svc.acquire_lease(&h.scope, &run, "w2").await.is_err(),
        "held lease cannot be acquired"
    );
    h.advance(TTL + 1);
    let rec = h.svc.recover(&h.scope, "sweeper").await.unwrap();
    assert!(
        rec.contains(&Recovered::TakenOver {
            run_id: run.clone(),
            status: RunStatus::Queued
        }),
        "{rec:?}"
    );
    let g2 = h.svc.acquire_lease(&h.scope, &run, "w2").await.unwrap();
    assert!(g2.fence.epoch > g1.fence.epoch);
}

#[tokio::test]
async fn takeover_redispatches_replay_safe_op_with_same_key() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let exec = executor(0);
    let actions = vec![op_with(|s| s.replay_safe = Some(true))];
    let d1 = crashing_driver(&h, actions, exec.clone());
    assert_eq!(
        d1.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Crashed
    );
    let op = h.rec(&run).await.state.active_op().cloned().unwrap();
    assert_eq!(exec.side_effects(&op.idempotency_key), 1);
    h.advance(TTL + 1);
    let d2 = h.driver(planner(vec![]), exec.clone());
    let out = d2.recover_and_drive(&h.scope, "w2").await.unwrap();
    assert_eq!(
        out,
        vec![(run.clone(), DriveOutcome::Exited(RunStatus::Queued))]
    );
    assert_eq!(
        exec.side_effects(&op.idempotency_key),
        1,
        "the side effect happened once"
    );
    assert_eq!(
        exec.started().iter().map(|(_, a)| *a).collect::<Vec<_>>(),
        vec![1, 2]
    );
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::LeaseTakenOver { .. }
        )),
        1
    );
    assert_eq!(
        count(
            &events,
            |k| matches!(k, RunEventKind::OperationStarted { attempt: 2, op_id, .. } if *op_id == op.op_id)
        ),
        1
    );
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationCompleted { .. }
        )),
        1
    );
}

struct NoResults;

#[async_trait]
impl ResultLoader for NoResults {
    async fn load_json(&self, _key: &str) -> Option<Value> {
        None
    }
}

#[tokio::test]
async fn takeover_abandons_non_replay_safe_mutation() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let g = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    let spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        replay_safe: Some(false),
        ..OperationSpec::default()
    };
    h.svc
        .begin_operation(&h.scope, &run, &g.fence, spec)
        .await
        .unwrap();
    h.advance(TTL + 1);
    let rec = h.svc.recover(&h.scope, "w2").await.unwrap();
    assert!(
        rec.contains(&Recovered::TakenOver {
            run_id: run.clone(),
            status: RunStatus::Queued
        }),
        "{rec:?}"
    );
    let events = h.events(&run).await;
    let abandoned = events
        .iter()
        .find(|e| matches!(e.kind, RunEventKind::OperationAbandoned { .. }))
        .expect("abandoned");
    let re = map_event(abandoned, &NoResults)
        .await
        .expect("delivered to a domain");
    assert_eq!(re.kind, EventKind::OperationFailed);
    assert_eq!(re.data["error_class"], "abandoned");
    assert_eq!(re.data["outcome_unknown"], true);
    assert!(h.rec(&run).await.state.active_op().is_none());
}

#[tokio::test]
async fn completed_op_is_not_redispatched_after_crash_between_commit_and_next() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let exec = executor(0);
    // w1 runs one op to completion (committed), then dies before its next step.
    let g = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    let spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        replay_safe: Some(true),
        ..OperationSpec::default()
    };
    let t = h
        .svc
        .begin_operation(&h.scope, &run, &g.fence, spec)
        .await
        .unwrap();
    let ctx = raisin_agent_runtime::driver::ExecContext {
        scope: h.scope.clone(),
        run_id: run.clone(),
        principal: t.record.principal.clone(),
        op: t.op.clone(),
        token: Default::default(),
        agent_ref: None,
        subject: None,
        executor_config: None,
    };
    let result = raisin_agent_runtime::driver::OperationExecutor::execute(exec.as_ref(), ctx).await;
    h.svc
        .finish_operation(&h.scope, &run, &g.fence, &t.op.op_id, result)
        .await
        .unwrap();
    assert!(h
        .svc
        .store()
        .idem_seen(&h.scope, &run, &t.op.idempotency_key)
        .await
        .unwrap()
        .is_some());
    h.advance(TTL + 1);
    let d2 = h.driver(planner(vec![]), exec.clone());
    d2.recover_and_drive(&h.scope, "w2").await.unwrap();
    d2.drive(&h.scope, &run, "w2").await.unwrap();
    assert_eq!(exec.started().len(), 1, "not executed again");
    assert_eq!(exec.total_side_effects(), 1);
}
