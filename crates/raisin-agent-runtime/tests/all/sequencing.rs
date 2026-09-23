//! Sequencing: seqs come from persisted state, contiguous from one.

use std::sync::Arc;

use raisin_agent_runtime::driver::NextAction;
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::ids::Seq;
use raisin_agent_runtime::lifecycle;
use raisin_agent_runtime::lifecycle::Fence;
use raisin_agent_runtime::service::ServiceError;
use raisin_agent_runtime::state::{RunOutcome, TerminalStatus};
use raisin_agent_runtime::store::{AgentRunStore, CommitRequest, StoreError};

use crate::common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seq_is_contiguous_from_one_across_controls_and_ops() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let mut actions: Vec<NextAction> = (0..5).map(|_| tool_op()).collect();
    actions.push(NextAction::Terminal {
        status: TerminalStatus::Completed,
        outcome: RunOutcome::new("succeeded", None),
    });
    let driver = h.driver(planner(actions), executor(1));
    let (d, s, r) = (driver.clone(), h.scope.clone(), run.clone());
    let drive = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    let mut tasks = Vec::new();
    for i in 0..16 {
        let (svc, s, r) = (h.svc.clone(), h.scope.clone(), run.clone());
        tasks.push(tokio::spawn(async move {
            svc.submit_control(&s, &r, steer(&format!("s{i}"), "x"), None)
                .await
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    drive.await.unwrap().unwrap();
    let events = h.events(&run).await;
    let seqs: Vec<u64> = events.iter().map(|e| e.seq.0).collect();
    assert_eq!(seqs, (1..=events.len() as u64).collect::<Vec<_>>());
    assert_eq!(h.rec(&run).await.last_seq, Seq(events.len() as u64));
    assert_eq!(
        count(&events, |k| matches!(
            k,
            RunEventKind::ControlReceived { .. }
        )),
        16
    );
}

#[tokio::test]
async fn read_events_after_seq_returns_suffix() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    for i in 0..5 {
        h.svc
            .submit_control(&h.scope, &run, steer(&format!("s{i}"), "x"), None)
            .await
            .unwrap();
    }
    let all = h.events(&run).await;
    let suffix = h
        .svc
        .read_events(&h.scope, &run, Seq(4), 100)
        .await
        .unwrap();
    assert_eq!(suffix, all[4..].to_vec());
    let page = h.svc.read_events(&h.scope, &run, Seq(0), 3).await.unwrap();
    assert_eq!(page, all[..3].to_vec());
}

#[tokio::test]
async fn commit_with_stale_version_conflicts_and_writes_nothing() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let rec = h.rec(&run).await;
    let t = lifecycle::apply_acquire(&rec, "w1", T0, TTL).unwrap();
    h.svc
        .submit_control(&h.scope, &run, steer("s1", "x"), None)
        .await
        .unwrap();
    let before = h.events(&run).await.len();
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
    assert!(matches!(err, StoreError::VersionConflict { .. }));
    assert_eq!(h.events(&run).await.len(), before);
}

#[tokio::test]
async fn failed_commit_assigns_no_seq() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let last = h.rec(&run).await.last_seq;
    h.store
        .fail_commits_when(Some(Arc::new(|req: &CommitRequest| {
            req.events
                .iter()
                .any(|e| matches!(e.kind, RunEventKind::SteerQueued { .. }))
        })));
    let err = h
        .svc
        .submit_control(&h.scope, &run, steer("s1", "x"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Store(StoreError::Backend(_))));
    assert_eq!(h.rec(&run).await.last_seq, last);
    h.store.fail_commits_when(None);
    h.svc
        .submit_control(&h.scope, &run, steer("s2", "x"), None)
        .await
        .unwrap();
    let events = h.events(&run).await;
    assert_eq!(
        events[last.0 as usize].seq,
        Seq(last.0 + 1),
        "no gap after the failed commit"
    );
}

#[tokio::test]
async fn subscriber_that_lags_rereads_without_loss() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let mut rx = h.svc.subscribe(&run);
    for i in 0..10 {
        h.svc
            .submit_control(&h.scope, &run, steer(&format!("s{i}"), "x"), None)
            .await
            .unwrap();
    }
    rx.changed().await.unwrap();
    let hint = *rx.borrow_and_update();
    assert_eq!(
        hint,
        h.rec(&run).await.last_seq,
        "the hint is the latest seq, not a count"
    );
    let events = h
        .svc
        .read_events(&h.scope, &run, Seq(0), 1000)
        .await
        .unwrap();
    assert_eq!(
        events.len() as u64,
        hint.0,
        "rereading from the cursor loses nothing"
    );
}
