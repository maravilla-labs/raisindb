//! The host (job step + sweeper) and the transport-neutral API.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::api::{self, Caller, ControlRequest, CreateRunRequest};
use raisin_agent_runtime::control::{authorize, ActorRef, ControlAck, ControlKind};
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use raisin_agent_runtime::driver::DriveOutcome;
use raisin_agent_runtime::events::OpOutcome;
use raisin_agent_runtime::host::{AgentRunHost, ReducerResolver};
use raisin_agent_runtime::ids::{Principal, PrincipalKind, RunScope, SubjectRef};
use raisin_agent_runtime::record::RunBudgets;
use raisin_agent_runtime::state::{RunOutcome, RunStatus, TerminalStatus};
use raisin_agent_runtime::testing::{ExecBehavior, RecordingExecutor};
use serde_json::json;

use crate::common::*;
use crate::resume::tool_then_complete;

/// Resolves every ref to one scripted reducer.
struct OneReducer(Arc<raisin_agent_runtime::testing::ScriptedReducer>);

#[async_trait]
impl ReducerResolver for OneReducer {
    async fn bind(&self, _: &RunScope, _: &str, _: &str) -> Result<ReducerRef, ReducerCallError> {
        Ok(self.0.reducer_ref().clone())
    }
    fn reducer(&self, _: &RunScope, _: &ReducerRef) -> Arc<dyn DomainReducer> {
        self.0.clone()
    }
}

fn host(h: &Harness) -> (AgentRunHost, Arc<RecordingExecutor>) {
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior {
        payload: Some(
            json!({ "envelope": "raisin.tool-result/1", "operation_id": "x", "status": "succeeded" }),
        ),
        ..ExecBehavior::default()
    }));
    let host = AgentRunHost::new(
        h.svc.clone(),
        Arc::new(OneReducer(tool_then_complete("h1"))),
        exec.clone(),
        "node-a",
    );
    (host, exec)
}

fn alice() -> Caller {
    Caller {
        id: "alice".into(),
        admin: false,
    }
}

fn create_req(reducer: bool) -> CreateRunRequest {
    serde_json::from_value(json!({
        "subject": { "workspace": "ws", "path": "/chat/1" },
        "input": { "text": "build it" },
        "reducer": if reducer { json!({ "function_path": "/lib/test/reducer" }) } else { json!(null) },
    }))
    .unwrap()
}

#[tokio::test]
async fn api_create_then_host_step_drives_a_domain_run_to_completion() {
    let h = Harness::manual();
    let (host, exec) = host(&h);
    let created = api::create(&host, "t1", "repo", &alice(), create_req(true))
        .await
        .unwrap();
    assert!(created.created);
    let out = host.step(&h.scope, &created.run_id, "job-1").await.unwrap();
    assert_eq!(out, DriveOutcome::Exited(RunStatus::Completed));
    assert_eq!(exec.total_side_effects(), 1);
    let view = api::get(&host, &h.scope, &created.run_id, &alice())
        .await
        .unwrap();
    assert_eq!(view.status, RunStatus::Completed);
    assert!(view.run.control_capability_hash.is_none());
    // Somebody else may not read it.
    let bob = Caller {
        id: "bob".into(),
        admin: false,
    };
    assert_eq!(
        api::get(&host, &h.scope, &created.run_id, &bob)
            .await
            .unwrap_err()
            .status,
        403
    );
}

#[tokio::test]
async fn host_leaves_client_driven_runs_to_their_driver() {
    let h = Harness::manual();
    let (host, exec) = host(&h);
    let created = api::create(&host, "t1", "repo", &alice(), create_req(false))
        .await
        .unwrap();
    let out = host.step(&h.scope, &created.run_id, "job-1").await.unwrap();
    assert_eq!(out, DriveOutcome::Exited(RunStatus::Queued));
    assert_eq!(exec.total_side_effects(), 0);
}

#[tokio::test]
async fn client_driver_runs_an_operation_and_completes_over_the_api() {
    let h = Harness::manual();
    let (host, _) = host(&h);
    let run = api::create(&host, "t1", "repo", &alice(), create_req(false))
        .await
        .unwrap()
        .run_id;
    let fence = api::acquire(&host, &h.scope, &run, &alice(), "client:alice:1")
        .await
        .unwrap();
    let begin: api::BeginRequest = serde_json::from_value(json!({
        "fence": fence, "kind": "tool_call", "input": { "tool": "edit_file" }
    }))
    .unwrap();
    let op = api::begin(&host, &h.scope, &run, &alice(), begin)
        .await
        .unwrap();
    let finish: api::FinishRequest = serde_json::from_value(json!({
        "fence": fence, "outcome": "succeeded", "payload": { "ok": true }
    }))
    .unwrap();
    assert_eq!(
        api::finish(&host, &h.scope, &run, &alice(), op.op_id.as_str(), finish)
            .await
            .unwrap(),
        RunStatus::Running
    );
    let done = api::CompleteRequest {
        fence: fence.clone(),
        status: TerminalStatus::Completed,
        outcome: RunOutcome::new("succeeded", Some("done".into())),
    };
    assert_eq!(
        api::complete(&host, &h.scope, &run, &alice(), done)
            .await
            .unwrap(),
        RunStatus::Completed
    );
    let events = api::events(&host, &h.scope, &run, &alice(), 0, 100)
        .await
        .unwrap();
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        raisin_agent_runtime::events::RunEventKind::OperationCompleted {
            outcome: OpOutcome::Succeeded,
            ..
        }
    )));
}

#[tokio::test]
async fn stop_over_the_api_is_idempotent_and_steer_after_stop_is_rejected() {
    let h = Harness::manual();
    let (host, _) = host(&h);
    let run = api::create(&host, "t1", "repo", &alice(), create_req(false))
        .await
        .unwrap()
        .run_id;
    let stop = || ControlRequest {
        control_id: "stop-1".into(),
        command: ControlKind::Stop {
            reason: Some("user".into()),
        },
        capability: None,
    };
    let first = api::control(&host, &h.scope, &run, &alice(), stop())
        .await
        .unwrap();
    assert!(matches!(first, ControlAck::Applied { .. }));
    let again = api::control(&host, &h.scope, &run, &alice(), stop())
        .await
        .unwrap();
    assert!(matches!(again, ControlAck::Duplicate { .. }));
    let steer = ControlRequest {
        control_id: "steer-1".into(),
        command: ControlKind::Steer {
            input: json!("more"),
        },
        capability: None,
    };
    let ack = api::control(&host, &h.scope, &run, &alice(), steer)
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Rejected { .. }));
}

/// The sweeper finalizes a terminal domain run whose `stopped` was never
/// delivered (the stop landed while no driver held it).
#[tokio::test]
async fn sweep_finalizes_a_stopped_domain_run() {
    let h = Harness::manual();
    let (host, _) = host(&h);
    let run = api::create(&host, "t1", "repo", &alice(), create_req(true))
        .await
        .unwrap()
        .run_id;
    let stop = ControlRequest {
        control_id: "s".into(),
        command: ControlKind::Stop { reason: None },
        capability: None,
    };
    api::control(&host, &h.scope, &run, &alice(), stop)
        .await
        .unwrap();
    let before = h.svc.get(&h.scope, &run).await.unwrap().unwrap();
    assert!(!before.domain.as_ref().unwrap().finalized);
    let report = host.sweep(&h.scope).await.unwrap();
    assert_eq!(report.finalized, 1, "{report:?}");
    let after = h.svc.get(&h.scope, &run).await.unwrap().unwrap();
    assert!(after.domain.unwrap().finalized);
    // Nothing left to do on the next pass.
    assert_eq!(host.sweep(&h.scope).await.unwrap().finalized, 0);
}

/// The sweeper drives a re-dispatched operation after a lease expiry.
#[tokio::test]
async fn sweep_takes_over_an_expired_lease_and_drives_the_redispatch() {
    let h = Harness::manual();
    let (host, exec) = host(&h);
    let run = api::create(&host, "t1", "repo", &alice(), create_req(true))
        .await
        .unwrap()
        .run_id;
    // A driver on another node starts the op and dies holding the lease.
    let g = h
        .svc
        .acquire_lease(&h.scope, &run, "dead-node")
        .await
        .unwrap();
    let reducer = tool_then_complete("h1");
    h.svc
        .domain_step(&h.scope, &run, &g.fence, reducer.as_ref())
        .await
        .unwrap();
    h.advance(TTL + 1);
    let report = host.sweep(&h.scope).await.unwrap();
    assert_eq!(report.redispatched, 1, "{report:?}");
    assert_eq!(exec.total_side_effects(), 1);
    let rec = h.svc.get(&h.scope, &run).await.unwrap().unwrap();
    assert_eq!(rec.state.status(), RunStatus::Completed);
}

/// A same-id actor of a different KIND is not the principal.
#[tokio::test]
async fn authorize_compares_principal_kind() {
    let h = Harness::manual();
    let mut req = h.request("/k");
    req.principal = Principal {
        kind: PrincipalKind::Agent,
        id: "bot".into(),
        on_behalf_of: Some("alice".into()),
    };
    let run = h.create_with(req).await;
    let rec = h.svc.get(&h.scope, &run).await.unwrap().unwrap();
    let as_user = ActorRef::user("bot");
    assert!(
        !authorize(&rec, &as_user, None),
        "a user named like the agent is not the agent"
    );
    let agent = ActorRef {
        kind: PrincipalKind::Agent,
        id: "bot".into(),
        capability: None,
    };
    assert!(authorize(&rec, &agent, None));
    assert!(
        authorize(&rec, &ActorRef::user("alice"), None),
        "the user it acts for"
    );
    let _ = SubjectRef {
        workspace: "w".into(),
        path: "/p".into(),
        node_id: None,
    };
    let _ = RunBudgets::default();
}

#[tokio::test]
async fn only_an_admin_caller_may_create_a_run_on_behalf_of_a_user() {
    let h = Harness::manual();
    let (host, _exec) = host(&h);
    let mut req = create_req(false);
    req.on_behalf_of = Some("carol".into());
    req.as_agent = Some("functions:/agents/helper".into());
    // A non-admin cannot act for someone else: the principal stays alice.
    let created = api::create(&host, "t1", "repo", &alice(), req.clone())
        .await
        .unwrap();
    let view = api::get(&host, &h.scope, &created.run_id, &alice())
        .await
        .unwrap();
    assert_eq!(view.run.principal.on_behalf_of.as_deref(), Some("alice"));
    // A system caller (a trigger) creates the run for the user who asked.
    let mut req2 = req;
    req2.subject.path = "/chat/2".into();
    let system = Caller {
        id: "system".into(),
        admin: true,
    };
    let created = api::create(&host, "t1", "repo", &system, req2)
        .await
        .unwrap();
    let carol = Caller {
        id: "carol".into(),
        admin: false,
    };
    let view = api::get(&host, &h.scope, &created.run_id, &carol)
        .await
        .unwrap();
    assert_eq!(view.run.principal.on_behalf_of.as_deref(), Some("carol"));
    assert_eq!(view.run.principal.id, "functions:/agents/helper");
}

#[tokio::test]
async fn by_subject_finds_every_run_of_a_conversation_newest_first() {
    let h = Harness::manual();
    let (host, _) = host(&h);
    let first = api::create(&host, "t1", "repo", &alice(), create_req(true))
        .await
        .unwrap();
    host.step(&h.scope, &first.run_id, "job-1").await.unwrap();
    h.advance(10);
    let second = api::create(&host, "t1", "repo", &alice(), create_req(false))
        .await
        .unwrap();
    let subject = SubjectRef {
        workspace: "ws".into(),
        path: "/chat/1".into(),
        node_id: None,
    };
    let runs = api::by_subject(&host, &h.scope, &alice(), &[subject.clone()], 10)
        .await
        .unwrap();
    let ids: Vec<_> = runs.iter().map(|v| v.run.run_id.clone()).collect();
    assert_eq!(
        ids,
        vec![second.run_id.clone(), first.run_id.clone()],
        "the live run first"
    );
    let bob = Caller {
        id: "bob".into(),
        admin: false,
    };
    assert!(api::by_subject(&host, &h.scope, &bob, &[subject], 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn follow_replays_from_a_seq_then_ends_with_the_run() {
    let h = Harness::manual();
    let (host, _) = host(&h);
    let created = api::create(&host, "t1", "repo", &alice(), create_req(true))
        .await
        .unwrap();
    host.step(&h.scope, &created.run_id, "job-1").await.unwrap();
    let (_stop_tx, stop) = tokio::sync::oneshot::channel();
    let mut got = Vec::new();
    api::follow(
        &host,
        &h.scope,
        &created.run_id,
        &alice(),
        1,
        stop,
        |item| {
            got.push(item);
            true
        },
    )
    .await
    .unwrap();
    let seqs: Vec<u64> = got
        .iter()
        .filter_map(|i| match i {
            api::FollowItem::Event(e) => Some(e.seq.0),
            _ => None,
        })
        .collect();
    assert_eq!(seqs.first(), Some(&2), "resumed after seq 1");
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "gap-free");
    assert!(matches!(got.last(), Some(api::FollowItem::End { .. })));
}
