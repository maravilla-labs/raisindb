//! Domain runs at the end: delivery order in the stop path, projections,
//! lease-free finalize, reducer availability and artifact pinning.

use std::sync::Arc;

use raisin_agent_contract::{EventKind, ItemStatus};
use raisin_agent_runtime::control::ControlKind;
use raisin_agent_runtime::domain::ReducerCallError;
use raisin_agent_runtime::driver::{DriveOutcome, Mode, RunDriver};
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::lifecycle;
use raisin_agent_runtime::state::{PauseReason, RunState, RunStatus};
use raisin_agent_runtime::store::CommitRequest;
use raisin_agent_runtime::testing::ScriptedReducer;
use serde_json::json;

use crate::common::*;
use crate::domain::{call, domain_run, resp};
use crate::resume::{reducer_ref, tool_then_complete};

#[tokio::test(start_paused = true)]
async fn op_completed_while_cancelling_is_delivered_before_stopped() {
    let h = Harness::tokio();
    let run = domain_run(&h, "h").await;
    let reducer = ScriptedReducer::new("h", |req| {
        let r = req.state_rev;
        Ok(match req.event.kind {
            EventKind::RunStarted => resp(r, true, vec![call("/lib/write", None, false)], None),
            _ => resp(r, true, vec![], None), // ingest, no effects
        })
    });
    let d = Arc::new(RunDriver::new(
        h.svc.clone(),
        Mode::Domain(reducer.clone()),
        executor(60_000),
    ));
    let (dd, s, rr) = (d.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { dd.drive(&s, &rr, "w").await });
    h.wait_operating(&run).await;
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap().unwrap(),
        DriveOutcome::Exited(RunStatus::Stopped)
    );
    let kinds: Vec<EventKind> = reducer.calls().iter().map(|c| c.event.kind).collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::RunStarted,
            EventKind::ToolResult,
            EventKind::OperationCancelled,
            EventKind::Stopped
        ]
    );
    let rec = h.rec(&run).await;
    assert!(rec.domain.as_ref().unwrap().finalized);
    assert_clean_terminal(&rec);
}

fn projecting_reducer() -> Arc<ScriptedReducer> {
    ScriptedReducer::new("h", |req| {
        let r = req.state_rev;
        let proj =
            json!({ "items": [{ "key": "a", "title": "Build A", "status": "in_progress" }] });
        Ok(match req.event.kind {
            EventKind::RunStarted => resp(r, true, vec![call("/lib/t", None, true)], Some(proj)),
            _ => resp(r, false, vec![], None),
        })
    })
}

#[tokio::test(start_paused = true)]
async fn terminal_commit_supersedes_stored_projection() {
    let h = Harness::tokio();
    let run = domain_run(&h, "h").await;
    let d = Arc::new(RunDriver::new(
        h.svc.clone(),
        Mode::Domain(projecting_reducer()),
        executor(60_000),
    ));
    let (dd, s, rr) = (d.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { dd.drive(&s, &rr, "w").await });
    h.wait_operating(&run).await;
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    let rec = h.rec(&run).await;
    assert!(
        rec.domain.as_ref().unwrap().projection_superseded,
        "no final projection was stored"
    );
    let p = h
        .svc
        .effective_projection(&h.scope, &run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p.items[0].status, ItemStatus::Stopped);
}

#[tokio::test(start_paused = true)]
async fn effective_projection_never_in_progress_unless_running() {
    let h = Harness::tokio();
    let run = domain_run(&h, "h").await;
    let d = Arc::new(RunDriver::new(
        h.svc.clone(),
        Mode::Domain(projecting_reducer()),
        executor(60_000),
    ));
    let (dd, s, rr) = (d.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { dd.drive(&s, &rr, "w").await });
    h.wait_operating(&run).await;
    let p = h
        .svc
        .effective_projection(&h.scope, &run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        p.items[0].status,
        ItemStatus::InProgress,
        "shown while running"
    );
    h.svc
        .submit_control(&h.scope, &run, cmd("p", ControlKind::Pause), None)
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Paused);
    let p = h
        .svc
        .effective_projection(&h.scope, &run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p.items[0].status, ItemStatus::Paused);
}

#[tokio::test]
async fn finalize_is_fenced_by_state_rev_and_runs_once() {
    let h = Harness::manual();
    let run = domain_run(&h, "h").await;
    let reducer = ScriptedReducer::new("h", |req| Ok(resp(req.state_rev, true, vec![], None)));
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap(); // terminal before any delivery
    let rec = h.rec(&run).await;
    // A stale finalizer (wrong state_rev) is fenced.
    let mut t = lifecycle::apply_checkpoint(
        &rec,
        raisin_agent_runtime::events::CheckpointReason::Periodic,
        None,
        T0,
    );
    t.record.domain.as_mut().unwrap().finalized = true;
    let stale = CommitRequest::from_transition(
        &h.scope,
        &t,
        lifecycle::Fence::DomainFinalize {
            expected_state_rev: 7,
        },
        T0,
    );
    assert!(matches!(
        raisin_agent_runtime::store::AgentRunStore::commit(h.store.as_ref(), stale).await,
        Err(raisin_agent_runtime::store::StoreError::FinalizeFenced)
    ));
    let (a, b) = tokio::join!(
        h.svc.finalize_domain(&h.scope, &run, reducer.as_ref()),
        h.svc.finalize_domain(&h.scope, &run, reducer.as_ref()),
    );
    a.unwrap();
    b.unwrap();
    assert!(h
        .svc
        .finalize_domain(&h.scope, &run, reducer.as_ref())
        .await
        .unwrap());
    let events = h.events(&run).await;
    let applied: Vec<u64> = events
        .iter()
        .filter_map(|e| match &e.kind {
            RunEventKind::DomainApplied { delivered_seq, .. } => Some(delivered_seq.0),
            _ => None,
        })
        .collect();
    let mut dedup = applied.clone();
    dedup.dedup();
    assert_eq!(applied, dedup, "each event committed once");
    assert!(h.rec(&run).await.domain.unwrap().finalized);
}

#[tokio::test]
async fn reducer_artifact_hash_change_pauses_with_reducer_changed() {
    let h = Harness::manual();
    let run = domain_run(&h, "h1").await;
    let changed = tool_then_complete("h2");
    let d = RunDriver::new(h.svc.clone(), Mode::Domain(changed.clone()), executor(0));
    assert_eq!(
        d.drive(&h.scope, &run, "w").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Paused)
    );
    let rec = h.rec(&run).await;
    assert!(
        matches!(&rec.state, RunState::Paused { reason: PauseReason::Reducer { reason }, .. } if reason == "reducer_changed")
    );
    assert_eq!(
        rec.domain
            .as_ref()
            .unwrap()
            .pending_artifact_hash
            .as_deref(),
        Some("h2")
    );
    assert!(
        changed.calls().is_empty(),
        "the changed reducer was never invoked"
    );
    let plain = cmd(
        "r1",
        ControlKind::Resume {
            budget_increase: None,
            accept_reducer_change: false,
        },
    );
    let ack = h
        .svc
        .submit_control(&h.scope, &run, plain, None)
        .await
        .unwrap();
    assert!(
        matches!(ack, raisin_agent_runtime::control::ControlAck::Rejected { ref reason, .. } if reason == "reducer_changed_requires_accept")
    );
    let accept = cmd(
        "r2",
        ControlKind::Resume {
            budget_increase: None,
            accept_reducer_change: true,
        },
    );
    h.svc
        .submit_control(&h.scope, &run, accept, None)
        .await
        .unwrap();
    assert_eq!(
        h.rec(&run).await.domain.unwrap().reducer.artifact_hash,
        "h2"
    );
    assert_eq!(
        d.drive(&h.scope, &run, "w").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Completed)
    );
}

#[tokio::test]
async fn reducer_unavailable_pauses_and_refusal_fails() {
    let h = Harness::manual();
    let run = domain_run(&h, "h").await;
    let down = ScriptedReducer::new("h", |_| Err(ReducerCallError::Unavailable("trap".into())));
    RunDriver::new(h.svc.clone(), Mode::Domain(down), executor(0))
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    assert!(
        matches!(h.rec(&run).await.state, RunState::Paused { reason: PauseReason::Reducer { ref reason }, .. } if reason == "reducer_unavailable")
    );
    let mut req = h.request("/other");
    req.reducer = Some(reducer_ref("h"));
    let run = h.create_with(req).await;
    let refusing = ScriptedReducer::new("h", |_| {
        Err(ReducerCallError::Refused {
            code: "nope".into(),
            message: String::new(),
        })
    });
    RunDriver::new(h.svc.clone(), Mode::Domain(refusing), executor(0))
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    let rec = h.rec(&run).await;
    assert_eq!(rec.state.status(), RunStatus::Failed);
    assert_eq!(rec.status_reason.as_deref(), Some("reducer_refused:nope"));
}

/// A stop that lands while NO driver holds the run (here: waiting on a
/// question) ends it without anyone to finalize the domain. The terminal
/// commit therefore wakes the run, and the woken driver finalizes it — so the
/// reducer does learn that the run stopped.
#[tokio::test(start_paused = true)]
async fn stop_while_waiting_wakes_the_run_for_finalize() {
    use raisin_agent_contract::EffectBody;
    use raisin_agent_runtime::state::WakeReason;
    let h = Harness::tokio();
    let run = domain_run(&h, "h").await;
    let reducer = ScriptedReducer::new("h", |req| {
        let r = req.state_rev;
        Ok(match req.event.kind {
            EventKind::RunStarted => resp(
                r,
                true,
                vec![EffectBody::AskUser {
                    question: "which?".into(),
                    choices: None,
                    schema: None,
                    expires_in_ms: None,
                }],
                None,
            ),
            _ => resp(r, true, vec![], None),
        })
    });
    let d = RunDriver::new(h.svc.clone(), Mode::Domain(reducer.clone()), executor(0));
    assert_eq!(
        d.drive(&h.scope, &run, "w").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Waiting)
    );
    h.waker.drain();
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    assert_eq!(
        h.waker.calls(),
        vec![(run.clone(), WakeReason::Finalize)],
        "the terminal commit of an unfinalized domain run wakes it"
    );
    // What the wake handler does: drive, which finalizes a terminal run.
    assert_eq!(
        d.drive(&h.scope, &run, "w2").await.unwrap(),
        DriveOutcome::NotAcquired
    );
    let kinds: Vec<EventKind> = reducer.calls().iter().map(|c| c.event.kind).collect();
    assert_eq!(kinds.last(), Some(&EventKind::Stopped));
    assert!(h.rec(&run).await.domain.unwrap().finalized);
}
