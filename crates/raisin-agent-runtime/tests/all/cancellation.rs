//! Stop, control dedup, authorization.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use raisin_agent_runtime::control::{ActorRef, ControlAck, ControlKind};
use raisin_agent_runtime::driver::{DriveOutcome, DriverHooks, HookAction, NextAction};
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::ids::RunId;
use raisin_agent_runtime::lifecycle::NewRequest;
use raisin_agent_runtime::record::{ActiveOperation, PendingKind};
use raisin_agent_runtime::service::AgentRunService;
use raisin_agent_runtime::state::{RunOutcome, RunStatus, TerminalStatus};

use crate::common::*;

#[tokio::test(start_paused = true)]
async fn stop_while_interruptible_op_running_cancels_token_and_ends_stopped() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let exec = executor(60_000);
    let driver = h.driver(planner(vec![tool_op(), tool_op()]), exec.clone());
    let (d, s, r) = (driver.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    h.wait_operating(&run).await;
    let ack = h
        .svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Applied { .. }));
    assert_eq!(
        task.await.unwrap().unwrap(),
        DriveOutcome::Exited(RunStatus::Stopped)
    );
    let types = h.types(&run).await;
    assert!(
        subsequence(
            &types,
            &[
                "control_received",
                "status_changed",
                "operation_cancelled",
                "terminal"
            ]
        ),
        "{types:?}"
    );
    let events = h.events(&run).await;
    assert!(events.iter().any(|e| matches!(
        e.kind,
        RunEventKind::StatusChanged {
            from: RunStatus::Running,
            to: RunStatus::Cancelling,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e.kind,
        RunEventKind::OperationCancelled {
            acknowledged: true,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e.kind,
        RunEventKind::Terminal {
            status: TerminalStatus::Stopped,
            ..
        }
    )));
    assert_eq!(
        exec.observed_cancel().len(),
        1,
        "the executor observed the cancellation"
    );
    assert_eq!(exec.total_side_effects(), 0);
    assert_clean_terminal(&h.rec(&run).await);
}

#[tokio::test(start_paused = true)]
async fn stop_while_non_interruptible_op_waits_then_stops_without_next_op() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let exec = executor(60_000);
    let driver = h.driver(
        planner(vec![op_with(|s| s.non_interruptible = true), tool_op()]),
        exec.clone(),
    );
    let (d, s, r) = (driver.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    h.wait_operating(&run).await;
    h.svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Cancelling);
    assert_eq!(
        task.await.unwrap().unwrap(),
        DriveOutcome::Exited(RunStatus::Stopped)
    );
    assert_eq!(exec.started().len(), 1, "no second operation began");
    assert_eq!(
        exec.total_side_effects(),
        1,
        "the non-interruptible op ran to its end"
    );
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationCompleted { .. }
        )),
        1
    );
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

struct StopAfterBegin {
    svc: Arc<AgentRunService>,
    scope: raisin_agent_runtime::ids::RunScope,
    run: Mutex<Option<RunId>>,
}

#[async_trait]
impl DriverHooks for StopAfterBegin {
    async fn after_begin_commit(&self, _op: &ActiveOperation) -> HookAction {
        let run = self.run.lock().unwrap().clone().unwrap();
        self.svc
            .submit_control(&self.scope, &run, stop("c-race"), None)
            .await
            .unwrap();
        HookAction::Continue
    }
}

#[tokio::test(start_paused = true)]
async fn stop_between_begin_commit_and_register_still_cancels() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let exec = executor(60_000);
    let hooks = Arc::new(StopAfterBegin {
        svc: h.svc.clone(),
        scope: h.scope.clone(),
        run: Mutex::new(Some(run.clone())),
    });
    let driver = raisin_agent_runtime::driver::RunDriver::new(
        h.svc.clone(),
        raisin_agent_runtime::driver::Mode::Planner(planner(vec![tool_op()])),
        exec.clone(),
    )
    .with_hooks(hooks);
    let outcome = driver.drive(&h.scope, &run, "w1").await.unwrap();
    assert_eq!(outcome, DriveOutcome::Exited(RunStatus::Stopped));
    assert_eq!(exec.total_side_effects(), 0, "the token was born cancelled");
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationCancelled {
                acknowledged: true,
                ..
            }
        )),
        1
    );
}

/// A stop that lands after `OperationStarted` but BEFORE the executor ran must
/// keep even a non-interruptible operation from running: "non-interruptible"
/// means "finishes once started", not "starts although stopped".
#[tokio::test(start_paused = true)]
async fn stop_before_execution_skips_non_interruptible_op() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let exec = executor(60_000);
    let hooks = Arc::new(StopAfterBegin {
        svc: h.svc.clone(),
        scope: h.scope.clone(),
        run: Mutex::new(Some(run.clone())),
    });
    let driver = raisin_agent_runtime::driver::RunDriver::new(
        h.svc.clone(),
        raisin_agent_runtime::driver::Mode::Planner(planner(vec![op_with(|s| {
            s.non_interruptible = true
        })])),
        exec.clone(),
    )
    .with_hooks(hooks);
    let outcome = driver.drive(&h.scope, &run, "w1").await.unwrap();
    assert_eq!(outcome, DriveOutcome::Exited(RunStatus::Stopped));
    assert!(exec.started().is_empty(), "the executor was never called");
    assert_eq!(exec.total_side_effects(), 0);
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::OperationCancelled {
                acknowledged: true,
                ..
            }
        )),
        1
    );
    assert_clean_terminal(&h.rec(&run).await);
}

#[tokio::test]
async fn stop_while_waiting_closes_pending_and_discards_steers() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let wait = NextAction::Wait(vec![NewRequest {
        kind: PendingKind::Input {
            prompt: "which?".into(),
            choices: None,
            schema: None,
        },
        effect_id: None,
        expires_at_ms: None,
    }]);
    let driver = h.driver(planner(vec![wait]), executor(0));
    assert_eq!(
        driver.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Waiting)
    );
    h.svc
        .submit_control(&h.scope, &run, steer("s1", "more"), None)
        .await
        .unwrap();
    assert_eq!(
        h.status(&run).await,
        RunStatus::Queued,
        "steer woke the waiting run"
    );
    assert_eq!(
        h.rec(&run).await.state.open_requests().len(),
        1,
        "open request kept"
    );
    h.svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    let events = h.events(&run).await;
    assert_eq!(
        count(
            &events,
            |k| matches!(k, RunEventKind::RequestClosed { reason, .. } if reason == "cancelled")
        ),
        1
    );
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::SteerDiscarded { .. }
        )),
        1
    );
    assert_clean_terminal(&h.rec(&run).await);
}

#[tokio::test]
async fn stop_is_idempotent_by_control_id() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let first = h
        .svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    let n = h.events(&run).await.len();
    let again = h
        .svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    assert_eq!(
        again,
        ControlAck::Duplicate {
            original_seq: first.seq()
        }
    );
    assert_eq!(h.events(&run).await.len(), n, "no new events");
}

#[tokio::test]
async fn control_id_reused_with_different_payload_is_rejected() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    h.svc
        .submit_control(&h.scope, &run, steer("c1", "a"), None)
        .await
        .unwrap();
    let ack = h
        .svc
        .submit_control(&h.scope, &run, steer("c1", "b"), None)
        .await
        .unwrap();
    assert!(
        matches!(ack, ControlAck::Rejected { ref reason, .. } if reason == "control_id_reused"),
        "{ack:?}"
    );
    let events = h.events(&run).await;
    assert_eq!(
        count(
            &events,
            |k| matches!(k, RunEventKind::ControlRejected { reason, .. } if reason == "control_id_reused")
        ),
        1
    );
    assert_eq!(
        h.rec(&run).await.steer_queue.len(),
        1,
        "the reused id changed nothing"
    );
}

#[tokio::test]
async fn control_from_unauthorized_actor_rejected() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let mut c = stop("c1");
    c.issued_by = ActorRef::user("mallory");
    let ack = h.svc.submit_control(&h.scope, &run, c, None).await.unwrap();
    assert!(matches!(ack, ControlAck::Rejected { ref reason, .. } if reason == "unauthorized"));
    assert_eq!(h.status(&run).await, RunStatus::Queued);
    assert!(
        h.types(&run)
            .await
            .contains(&"control_rejected".to_string()),
        "the rejection is logged"
    );
}

#[tokio::test]
async fn control_with_capability_accepted() {
    let h = Harness::manual();
    let mut req = h.request("/a");
    req.control_capability = Some("s3cret".into());
    let run = h.create_with(req).await;
    let mut c = stop("c1");
    c.issued_by = ActorRef {
        capability: Some("s3cret".into()),
        ..ActorRef::user("stream-client")
    };
    let ack = h.svc.submit_control(&h.scope, &run, c, None).await.unwrap();
    assert!(matches!(ack, ControlAck::Applied { .. }), "{ack:?}");
    let logged = serde_json::to_string(&h.events(&run).await).unwrap();
    assert!(
        !logged.contains("s3cret"),
        "the capability never enters the log"
    );
}

#[tokio::test]
async fn control_after_terminal_is_rejected_and_logged() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    h.svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    let ack = h
        .svc
        .submit_control(&h.scope, &run, cmd("c2", ControlKind::Pause), None)
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Rejected { ref reason, .. } if reason == "run_terminal"));
    let events = h.events(&run).await;
    assert!(matches!(
        events.last().unwrap().kind,
        RunEventKind::ControlRejected { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn stop_does_not_block_behind_running_op() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let exec = executor(180_000);
    let driver = h.driver(
        planner(vec![op_with(|s| s.non_interruptible = true)]),
        exec.clone(),
    );
    let (d, s, r) = (driver.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    h.wait_operating(&run).await;
    let before = tokio::time::Instant::now();
    h.svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    assert!(
        before.elapsed() < std::time::Duration::from_secs(1),
        "the stop did not wait for the model call"
    );
    assert_eq!(
        exec.total_side_effects(),
        0,
        "the operation is still running"
    );
    assert_eq!(h.status(&run).await, RunStatus::Cancelling);
    task.await.unwrap().unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Stopped);
}

#[tokio::test]
async fn system_principal_requires_a_system_token() {
    let h = Harness::manual();
    let mut req = h.request("/sys");
    req.principal.kind = raisin_agent_runtime::ids::PrincipalKind::System;
    assert!(h.svc.create(req.clone(), None).await.is_err());
    let token = raisin_agent_runtime::ids::SystemToken::in_process();
    assert!(h.svc.create(req, Some(&token)).await.is_ok());
    let _ = RunOutcome::default();
}
