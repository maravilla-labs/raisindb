//! Child runs: admission, the durable mailbox, interrupt propagation, budgets.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::child::SpawnChild;
use raisin_agent_runtime::control::{ActorRef, ControlAck};
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use raisin_agent_runtime::events::{OpOutcome, RunEventKind};
use raisin_agent_runtime::host::{AgentRunHost, ReducerResolver};
use raisin_agent_runtime::ids::{RunId, RunScope};
use raisin_agent_runtime::lifecycle::{NewRequest, OperationResult, OperationSpec};
use raisin_agent_runtime::record::{BudgetPolicy, PendingKind, RunBudgets};
use raisin_agent_runtime::service::ServiceError;
use raisin_agent_runtime::service_child::SpawnOutcome;
use raisin_agent_runtime::service_child_ctl::{ChildAction, InterruptMode};
use raisin_agent_runtime::state::{RunOutcome, RunStatus, TerminalStatus, WakeReason};
use raisin_agent_runtime::store::AgentRunStore;
use serde_json::{json, Value};

use crate::common::*;

/// No reducers: every run here is client-driven.
struct NoReducers;

#[async_trait]
impl ReducerResolver for NoReducers {
    async fn bind(&self, _: &RunScope, p: &str, _: &str) -> Result<ReducerRef, ReducerCallError> {
        Err(ReducerCallError::Unavailable(p.into()))
    }
    fn reducer(&self, _: &RunScope, _: &ReducerRef) -> Arc<dyn DomainReducer> {
        unreachable!("no domain runs in these tests")
    }
}

pub fn host(h: &Harness) -> AgentRunHost {
    AgentRunHost::new(h.svc.clone(), Arc::new(NoReducers), executor(0), "node-a")
}

pub fn alice() -> ActorRef {
    ActorRef::user("alice")
}

pub fn spawn_req(title: &str, extra: Value) -> SpawnChild {
    let mut v = json!({ "objective": { "title": title, "instructions": "do it" } });
    if let (Some(obj), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in extra {
            if k == "objective" {
                for (ok, ov) in val.as_object().unwrap() {
                    obj["objective"][ok] = ov.clone();
                }
            } else {
                obj.insert(k.clone(), val.clone());
            }
        }
    }
    serde_json::from_value(v).unwrap()
}

pub async fn spawn(h: &Harness, parent: &RunId, req: SpawnChild) -> SpawnOutcome {
    h.svc
        .spawn_child(&h.scope, parent, req, &alice(), None)
        .await
        .expect("spawn")
}

/// Run every recorded wake as a job step, until none is left.
pub async fn pump(h: &Harness, host: &AgentRunHost) {
    for _ in 0..50 {
        let wakes = h.waker.drain();
        if wakes.is_empty() {
            return;
        }
        for (run, _) in wakes {
            host.step(&h.scope, &run, "job").await.unwrap();
        }
    }
    panic!("wakes never settled");
}

pub async fn finish_client(h: &Harness, run: &RunId, status: TerminalStatus, outcome: RunOutcome) {
    let g = h.svc.acquire_lease(&h.scope, run, "client").await.unwrap();
    h.svc
        .complete(&h.scope, run, &g.fence, status, outcome, None)
        .await
        .unwrap();
}

pub fn succeeded(detail: Value) -> RunOutcome {
    RunOutcome {
        kind: "succeeded".into(),
        detail: Some(detail),
        ..RunOutcome::default()
    }
}

fn child_wait(child: &RunId) -> NewRequest {
    NewRequest {
        kind: PendingKind::Child {
            child_run_id: child.clone(),
        },
        effect_id: None,
        expires_at_ms: None,
    }
}

#[tokio::test]
async fn child_completion_reaches_parent_mailbox_across_a_worker_crash() {
    let h = Harness::manual();
    let host = host(&h);
    let parent = h.create("/p").await;
    let req = spawn_req(
        "schema",
        json!({ "objective": { "hand_back": { "required_fields": ["summary"] } } }),
    );
    let child = spawn(&h, &parent, req).await.child_run_id;
    let crec = h.rec(&child).await;
    assert_eq!(
        (
            crec.parent_run_id.as_ref(),
            crec.root_run_id.as_ref(),
            crec.depth
        ),
        (Some(&parent), Some(&parent), 1)
    );

    // The parent waits for the child under its lease.
    let pg = h
        .svc
        .acquire_lease(&h.scope, &parent, "parent")
        .await
        .unwrap();
    h.svc
        .wait(&h.scope, &parent, &pg.fence, vec![child_wait(&child)])
        .await
        .unwrap();
    assert_eq!(h.status(&parent).await, RunStatus::Waiting);

    finish_client(
        &h,
        &child,
        TerminalStatus::Completed,
        succeeded(json!({ "summary": "done" })),
    )
    .await;
    // Crash 1: the worker dies before the Handback wake is ever run.
    let lost = h.waker.drain();
    assert!(lost.contains(&(child.clone(), WakeReason::Handback)));
    assert!(h.rec(&parent).await.mailbox.is_empty());
    assert_eq!(
        h.store.scan_handback_owed(&h.scope, 10).await.unwrap(),
        vec![child.clone()]
    );

    // Crash 2: a later attempt lands the parent commit, then dies before the
    // child is marked delivered.
    h.store.fail_commits_when(Some(Arc::new(|r| {
        r.events
            .iter()
            .any(|e| matches!(e.kind, RunEventKind::HandbackDelivered { .. }))
    })));
    assert!(h.svc.deliver_handback(&h.scope, &child).await.is_err());
    h.store.fail_commits_when(None);
    assert_eq!(h.rec(&parent).await.mailbox.len(), 1);

    // Any node's sweeper finishes it; the redelivery is a no-op on the parent.
    let report = host.sweep(&h.scope).await.unwrap();
    assert_eq!(report.handbacks, 1);
    let prec = h.rec(&parent).await;
    assert_eq!(prec.mailbox.len(), 1);
    let handbacks = count(&h.events(&parent).await, |k| {
        matches!(k, RunEventKind::ChildHandback { .. })
    });
    assert_eq!(handbacks, 1);
    assert!(h
        .store
        .scan_handback_owed(&h.scope, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(h.rec(&child).await.delegation.unwrap().handback_delivered);
    // The durable mailbox resolved the wait: no polling of anything transient.
    assert_eq!(prec.state.status(), RunStatus::Queued);
    assert!(prec.children[0].delivered);
    let mb = h.svc.mailbox(&h.scope, &parent).await.unwrap();
    assert_eq!(mb[0]["item"]["kind"], "completion");
    assert_eq!(mb[0]["payload"]["status"], "succeeded");
    assert_eq!(mb[0]["payload"]["payload"]["contract"]["satisfied"], true);
    assert_eq!(
        h.svc
            .ack_mailbox(&h.scope, &parent, 1, &alice(), None)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_handback_violating_the_contract_is_marked_not_satisfied() {
    let h = Harness::manual();
    let host = host(&h);
    let parent = h.create("/p").await;
    let req = spawn_req(
        "app",
        json!({ "objective": {
            "expected_artifacts": [{ "kind": "appdef" }],
            "acceptance_checks": [{ "id": "board-renders" }],
            "hand_back": { "required_fields": ["summary"] },
        } }),
    );
    let child = spawn(&h, &parent, req).await.child_run_id;
    finish_client(
        &h,
        &child,
        TerminalStatus::Completed,
        succeeded(json!({ "checks": [{ "id": "board-renders", "passed": false }] })),
    )
    .await;
    pump(&h, &host).await;
    let mb = h.svc.mailbox(&h.scope, &parent).await.unwrap();
    let env = &mb[0]["payload"];
    assert_eq!(env["status"], "blocked");
    let violations = env["payload"]["contract"]["violations"].as_array().unwrap();
    assert_eq!(violations.len(), 3, "{violations:?}");
}

#[tokio::test]
async fn interrupt_propagates_to_children_and_grandchildren() {
    let h = Harness::manual();
    let host = host(&h);
    let parent = h.create("/p").await;
    let child = spawn(&h, &parent, spawn_req("a", json!({})))
        .await
        .child_run_id;
    let grandchild = spawn(&h, &child, spawn_req("a.1", json!({})))
        .await
        .child_run_id;
    assert_eq!(h.rec(&grandchild).await.root_run_id, Some(parent.clone()));
    assert_eq!(h.rec(&grandchild).await.depth, 2);
    let other = spawn(&h, &parent, spawn_req("b", json!({})))
        .await
        .child_run_id;

    // An operating child: the interrupt cancels its in-flight operation.
    let g = h
        .svc
        .acquire_lease(&h.scope, &other, "client")
        .await
        .unwrap();
    let op = h
        .svc
        .begin_operation(&h.scope, &other, &g.fence, OperationSpec::default())
        .await
        .unwrap()
        .op;
    let interrupt = ChildAction::Interrupt {
        mode: InterruptMode::Stop,
        reason: Some("plan changed".into()),
    };
    let ack = h
        .svc
        .control_child(
            &h.scope,
            &parent,
            &other,
            interrupt.clone(),
            "i1",
            &alice(),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Applied { .. }));
    assert_eq!(h.status(&other).await, RunStatus::Cancelling);
    assert!(h.svc.cancels().register(&other, &op.op_id).is_cancelled());
    let cancelled = OperationResult {
        outcome: Some(OpOutcome::Cancelled),
        ..Default::default()
    };
    h.svc
        .finish_operation(&h.scope, &other, &g.fence, &op.op_id, cancelled)
        .await
        .unwrap();
    assert_eq!(h.status(&other).await, RunStatus::Stopped);
    // A retry of the same interrupt is deduplicated.
    let again = h
        .svc
        .control_child(&h.scope, &parent, &other, interrupt, "i1", &alice(), None)
        .await
        .unwrap();
    assert!(matches!(again, ControlAck::Duplicate { .. }));
    pump(&h, &host).await;
    let mb = h.svc.mailbox(&h.scope, &parent).await.unwrap();
    assert_eq!(mb.len(), 1);
    assert_eq!(mb[0]["item"]["status"], "stopped");
    assert_eq!(mb[0]["payload"]["status"], "failed");

    // Stopping the parent reaches the whole subtree, even when the cascade
    // wake is lost: the sweeper stops the orphan, its own cascade follows.
    h.svc
        .submit_control(&h.scope, &parent, stop("s1"), None)
        .await
        .unwrap();
    assert_eq!(h.status(&parent).await, RunStatus::Stopped);
    let lost = h.waker.drain();
    assert!(lost.contains(&(parent.clone(), WakeReason::Cascade)));
    let report = host.sweep(&h.scope).await.unwrap();
    assert!(report.cascaded >= 1);
    pump(&h, &host).await;
    host.sweep(&h.scope).await.unwrap();
    pump(&h, &host).await;
    assert_eq!(h.status(&child).await, RunStatus::Stopped);
    assert_eq!(h.status(&grandchild).await, RunStatus::Stopped);
    assert_eq!(
        h.rec(&grandchild).await.status_reason.as_deref(),
        Some("user_stop")
    );
    // Every child still handed back, to a terminal parent.
    let prec = h.rec(&parent).await;
    assert!(
        prec.children.iter().all(|l| l.delivered),
        "{:?}",
        prec.children
    );
    assert!(h
        .store
        .scan_handback_owed(&h.scope, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn budget_exhaustion_fails_the_child_honestly() {
    let h = Harness::manual();
    let host = host(&h);
    let mut req = h.request("/p");
    req.budgets = RunBudgets {
        max_operations: Some(10),
        ..RunBudgets::default()
    };
    let parent = h.create_with(req).await;
    let out = spawn(
        &h,
        &parent,
        spawn_req("tight", json!({ "budgets": { "max_operations": 1 } })),
    )
    .await;
    assert_eq!(out.budgets.max_operations, Some(1));
    assert_eq!(out.budgets.on_exceeded, BudgetPolicy::Fail);
    let report = h
        .svc
        .spawn_child(
            &h.scope,
            &parent,
            spawn_req("default", json!({})),
            &alice(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        report.budgets.max_operations,
        Some(5),
        "an unspecified budget gets half the spare"
    );
    let prec = h.rec(&parent).await;
    assert_eq!(
        raisin_agent_runtime::budget::spare(&prec, T0).operations,
        Some(4)
    );

    let child = out.child_run_id;
    h.driver(planner(vec![tool_op(), tool_op()]), executor(0))
        .drive(&h.scope, &child, "w")
        .await
        .unwrap();
    let crec = h.rec(&child).await;
    assert_eq!(crec.state.status(), RunStatus::Failed);
    assert_eq!(
        crec.status_reason.as_deref(),
        Some("budget_exceeded:max_operations")
    );
    assert_clean_terminal(&crec);

    pump(&h, &host).await;
    let mb = h.svc.mailbox(&h.scope, &parent).await.unwrap();
    assert_eq!(mb[0]["payload"]["status"], "failed");
    assert_eq!(
        mb[0]["payload"]["diagnostics"][0]["code"],
        "budget_exceeded:max_operations"
    );
    let prec = h.rec(&parent).await;
    assert_eq!(prec.usage.child_operations, 1);
    // The reservation is released; what the child used stays counted.
    assert_eq!(
        raisin_agent_runtime::budget::spare(&prec, T0).operations,
        Some(4)
    );

    // A parent with nothing left cannot lend anything.
    let mut req = h.request("/broke");
    req.budgets = RunBudgets {
        max_operations: Some(0),
        ..RunBudgets::default()
    };
    let broke = h.create_with(req).await;
    let err = h
        .svc
        .spawn_child(&h.scope, &broke, spawn_req("x", json!({})), &alice(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ServiceError::Refused(r) if r.code == "child_budget_exceeded:max_operations"),
        "{err:?}"
    );
}
