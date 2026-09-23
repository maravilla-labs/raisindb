//! The core half of "no stopped run displays an active task".

use raisin_agent_runtime::control::{ApprovalDecision, ControlKind};
use raisin_agent_runtime::driver::{Mode, NextAction, RunDriver};
use raisin_agent_runtime::events::OpOutcome;
use raisin_agent_runtime::ids::{RequestId, RunId};
use raisin_agent_runtime::lifecycle::{self, Fence, NewRequest, OperationResult, OperationSpec};
use raisin_agent_runtime::record::{check_invariants, BudgetPolicy, PendingKind, RunBudgets};
use raisin_agent_runtime::service::ServiceError;
use raisin_agent_runtime::state::{RunOutcome, RunStatus, TerminalStatus};
use raisin_agent_runtime::store::{AgentRunStore, CommitRequest, StoreError};

use crate::common::*;
use crate::resume::{reducer_ref, tool_then_complete};

#[tokio::test]
async fn terminal_record_has_no_lease_op_open_requests_or_steers() {
    let h = Harness::manual();
    // stop from Waiting with a queued steer
    let run = h.create("/stop").await;
    let wait = NextAction::Wait(vec![NewRequest {
        kind: PendingKind::Input {
            prompt: "?".into(),
            choices: None,
            schema: None,
        },
        effect_id: None,
        expires_at_ms: None,
    }]);
    h.driver(planner(vec![wait]), executor(0))
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    h.svc
        .submit_control(&h.scope, &run, steer("s", "x"), None)
        .await
        .unwrap();
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    assert_clean_terminal(&h.rec(&run).await);
    // complete
    let run = h.create("/complete").await;
    let done = NextAction::Terminal {
        status: TerminalStatus::Completed,
        outcome: RunOutcome::new("succeeded", None),
    };
    h.driver(planner(vec![tool_op(), done]), executor(0))
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Completed);
    assert_clean_terminal(&h.rec(&run).await);
    // domain complete
    let mut req = h.request("/domain");
    req.reducer = Some(reducer_ref("h"));
    let run = h.create_with(req).await;
    RunDriver::new(
        h.svc.clone(),
        Mode::Domain(tool_then_complete("h")),
        executor(0),
    )
    .drive(&h.scope, &run, "w")
    .await
    .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Completed);
    assert_clean_terminal(&h.rec(&run).await);
    // budget fail
    let mut req = h.request("/budget");
    req.budgets = RunBudgets {
        max_operations: Some(1),
        on_exceeded: BudgetPolicy::Fail,
        ..RunBudgets::default()
    };
    let run = h.create_with(req).await;
    h.driver(planner(vec![tool_op(), tool_op()]), executor(0))
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Failed);
    assert_eq!(
        h.rec(&run).await.status_reason.as_deref(),
        Some("budget_exceeded:max_operations")
    );
    assert_clean_terminal(&h.rec(&run).await);
    // recovery to stopped
    let run = h.create("/recover").await;
    let g = h.svc.acquire_lease(&h.scope, &run, "w").await.unwrap();
    h.svc
        .begin_operation(&h.scope, &run, &g.fence, OperationSpec::default())
        .await
        .unwrap();
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    h.advance(TTL + 1);
    h.svc.recover(&h.scope, "w2").await.unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Stopped);
    assert_clean_terminal(&h.rec(&run).await);
}

#[tokio::test]
async fn store_refuses_record_violating_checker_invariants() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let rec = h.rec(&run).await;
    let mut t = lifecycle::apply_acquire(&rec, "w1", T0, TTL).unwrap();
    t.record.lease_epoch = raisin_agent_runtime::ids::LeaseEpoch(t.record.lease_epoch.0 + 5); // lease epoch != record epoch (I1)
    let err = h
        .store
        .commit(CommitRequest::from_transition(
            &h.scope,
            &t,
            Fence::None,
            T0,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Invariant(ref v) if v.code == "I1"),
        "{err:?}"
    );
    let good = lifecycle::apply_checkpoint(
        &rec,
        raisin_agent_runtime::events::CheckpointReason::Periodic,
        None,
        T0,
    );
    h.store
        .commit(CommitRequest::from_transition(
            &h.scope,
            &good,
            Fence::None,
            T0,
        ))
        .await
        .unwrap();
    let mut back = lifecycle::apply_checkpoint(
        &good.record,
        raisin_agent_runtime::events::CheckpointReason::Periodic,
        None,
        T0,
    );
    back.record.counters.checkpoint = 0;
    back.checkpoint = None;
    let err = h
        .store
        .commit(CommitRequest::from_transition(
            &h.scope,
            &back,
            Fence::None,
            T0,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Invariant(ref v) if v.code == "I3"),
        "{err:?}"
    );
}

/// A small deterministic generator (no new dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

#[tokio::test]
async fn property_random_control_interleavings_preserve_invariants() {
    for seed in 1..=40u64 {
        let h = Harness::manual();
        let mut rng = Lcg(seed);
        let run: RunId = h.create("/p").await;
        let mut fence = None;
        let mut op = None;
        for step in 0..60 {
            let id = format!("c{seed}-{step}");
            let res: Result<(), ServiceError> = match rng.next(11) {
                0 => h
                    .svc
                    .acquire_lease(&h.scope, &run, "w")
                    .await
                    .map(|g| fence = Some(g.fence)),
                1 => match &fence {
                    Some(f) => h
                        .svc
                        .begin_operation(
                            &h.scope,
                            &run,
                            f,
                            OperationSpec {
                                replay_safe: Some(rng.next(2) == 0),
                                ..OperationSpec::default()
                            },
                        )
                        .await
                        .map(|t| op = Some(t.op.op_id)),
                    None => Ok(()),
                },
                2 => match (&fence, &op) {
                    (Some(f), Some(o)) => {
                        let outcome = [OpOutcome::Succeeded, OpOutcome::Failed, OpOutcome::Waiting]
                            [rng.next(3) as usize];
                        let r = OperationResult {
                            outcome: Some(outcome),
                            resume_key: Some("k".into()),
                            ..OperationResult::default()
                        };
                        h.svc
                            .finish_operation(&h.scope, &run, f, o, r)
                            .await
                            .map(|_| ())
                    }
                    _ => Ok(()),
                },
                3 => h
                    .svc
                    .submit_control(&h.scope, &run, stop(&id), None)
                    .await
                    .map(|_| ()),
                4 => h
                    .svc
                    .submit_control(&h.scope, &run, cmd(&id, ControlKind::Pause), None)
                    .await
                    .map(|_| ()),
                5 => h
                    .svc
                    .submit_control(
                        &h.scope,
                        &run,
                        cmd(
                            &id,
                            ControlKind::Resume {
                                budget_increase: None,
                                accept_reducer_change: false,
                            },
                        ),
                        None,
                    )
                    .await
                    .map(|_| ()),
                6 => h
                    .svc
                    .submit_control(&h.scope, &run, steer(&id, "x"), None)
                    .await
                    .map(|_| ()),
                7 => {
                    let kind = ControlKind::Approve {
                        request_id: RequestId(format!("{run}/req/1")),
                        decision: ApprovalDecision::Approve,
                        subject_digest: "d".into(),
                    };
                    h.svc
                        .submit_control(&h.scope, &run, cmd(&id, kind), None)
                        .await
                        .map(|_| ())
                }
                8 => {
                    h.advance(rng.next(2) * (TTL + 1));
                    h.svc.recover(&h.scope, "sweeper").await.map(|_| ())
                }
                9 => match &fence {
                    Some(f) => h
                        .svc
                        .release_lease(&h.scope, &run, f, true)
                        .await
                        .map(|_| ()),
                    None => Ok(()),
                },
                _ => match &fence {
                    Some(f) => h.svc.consume_steers(&h.scope, &run, f).await.map(|_| ()),
                    None => Ok(()),
                },
            };
            if let Err(ServiceError::Store(StoreError::Invariant(v))) = &res {
                panic!("seed {seed} step {step}: invariant violated: {v}");
            }
            let rec = h.rec(&run).await;
            check_invariants(None, &rec).unwrap_or_else(|v| panic!("seed {seed} step {step}: {v}"));
            if rec.state.is_terminal() {
                assert_clean_terminal(&rec);
            }
            let events = h.events(&run).await;
            assert_eq!(events.last().unwrap().seq, rec.last_seq, "I2");
        }
    }
}
