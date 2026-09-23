// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The store conformance suite: every [`AgentRunStore`] backend must pass it.
//!
//! Run it from a backend's tests with a factory returning a FRESH store per
//! call. Panics (with the check's name) on the first failure.

use std::sync::Arc;

use serde_json::json;

use crate::control::{ActorRef, ControlCommand, ControlKind};
use crate::events::{CheckpointReason, RunEventKind};
use crate::ids::{ControlId, Principal, RunId, RunScope, Seq, SubjectRef};
use crate::lifecycle::{self, apply_control, Fence, LeaseFence};
use crate::record::AgentRunRecord;
use crate::service::{new_record, CreateRun};
use crate::state::{RunOutcome, RunStatus, TerminalStatus};
use crate::store::{AgentRunStore, CommitRequest, CreateOutcome, StoreError};
use crate::tx::Transition;

const T0: u64 = 1_000_000;

/// A scope for tests.
pub fn scope(tenant: &str) -> RunScope {
    RunScope::new(tenant, "repo", "main")
}

/// A create request for `subject_path` in `scope`.
pub fn create_request(scope: &RunScope, subject_path: &str) -> CreateRun {
    CreateRun {
        scope: scope.clone(),
        subject: SubjectRef {
            workspace: "ws".into(),
            path: subject_path.into(),
            node_id: None,
        },
        principal: Principal::user("alice"),
        control_capability: None,
        agent_ref: None,
        create_key: None,
        budgets: Default::default(),
        input: json!({ "text": "hello" }),
        reducer: None,
        executor_config: None,
        waiter: None,
    }
}

pub(crate) fn first_event(req: &CreateRun) -> RunEventKind {
    RunEventKind::RunCreated {
        subject: req.subject.clone(),
        principal: req.principal.clone(),
        budgets: req.budgets.clone(),
        input: req.input.clone(),
    }
}

pub(crate) async fn create(store: &dyn AgentRunStore, req: &CreateRun) -> AgentRunRecord {
    let rec = new_record(req, RunId::new_v4(), T0);
    let run = rec.run_id.clone();
    let out = store
        .create(rec, first_event(req), req.create_key.as_deref())
        .await
        .expect("create");
    assert_eq!(
        out,
        CreateOutcome::Created {
            run_id: run.clone(),
            seq: Seq(1)
        },
        "create outcome"
    );
    store
        .load(&req.scope, &run)
        .await
        .unwrap()
        .expect("created record loads")
}

pub(crate) async fn commit(
    store: &dyn AgentRunStore,
    t: &Transition,
    fence: Fence,
) -> Result<crate::store::CommitOutcome, StoreError> {
    store
        .commit(CommitRequest::from_transition(
            &t.record.scope,
            t,
            fence,
            T0 + 10,
        ))
        .await
}

fn stop(id: &str) -> ControlCommand {
    ControlCommand {
        control_id: ControlId(id.into()),
        kind: ControlKind::Stop { reason: None },
        issued_by: ActorRef::user("alice"),
        at_ms: T0,
    }
}

/// Run every check against fresh stores from `make`.
pub async fn conformance_suite<F>(make: F)
where
    F: Fn() -> Arc<dyn AgentRunStore>,
{
    create_and_load(make().as_ref()).await;
    admission(make().as_ref()).await;
    commit_semantics(make().as_ref()).await;
    fence_and_invariants(make().as_ref()).await;
    control_dedup(make().as_ref()).await;
    side_tables(make().as_ref()).await;
    status_index_and_terminal(make().as_ref()).await;
    scopes_are_isolated(make().as_ref()).await;
    crate::conformance_child::lineage_tables(make().as_ref()).await;
}

async fn create_and_load(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let rec = create(store, &create_request(&s, "/a")).await;
    assert_eq!(rec.version.0, 1, "create_and_load: version");
    let events = store
        .read_events(&s, &rec.run_id, Seq(0), 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 1, "create_and_load: one event");
    assert!(
        matches!(events[0].kind, RunEventKind::RunCreated { .. }),
        "create_and_load: RunCreated"
    );
    assert_eq!(events[0].seq, Seq(1));
}

async fn admission(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let req = create_request(&s, "/subject");
    let first = create(store, &req).await;
    let again = store
        .create(
            new_record(&req, RunId::new_v4(), T0),
            first_event(&req),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        again,
        CreateOutcome::Existing {
            run_id: first.run_id.clone(),
            status: RunStatus::Queued
        },
        "admission: same subject"
    );
    let mut keyed = create_request(&s, "/other");
    keyed.create_key = Some("msg-1".into());
    let k1 = create(store, &keyed).await;
    let mut keyed2 = create_request(&s, "/third");
    keyed2.create_key = Some("msg-1".into());
    let k2 = store
        .create(
            new_record(&keyed2, RunId::new_v4(), T0),
            first_event(&keyed2),
            Some("msg-1"),
        )
        .await
        .unwrap();
    assert_eq!(
        k2,
        CreateOutcome::Existing {
            run_id: k1.run_id,
            status: RunStatus::Queued
        },
        "admission: create key"
    );
}

async fn commit_semantics(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let rec = create(store, &create_request(&s, "/c")).await;
    let t = lifecycle::apply_acquire(&rec, "w1", T0, 90_000).unwrap();
    let out = commit(store, &t, Fence::None)
        .await
        .expect("acquire commits");
    assert_eq!(out.first_seq, Seq(2), "commit: first seq");
    assert_eq!(out.last_seq, t.record.last_seq, "commit: last seq");
    // Stale version: nothing written.
    let stale = commit(store, &t, Fence::None).await;
    assert!(
        matches!(stale, Err(StoreError::VersionConflict { .. })),
        "commit: stale version conflicts"
    );
    let events = store
        .read_events(&s, &rec.run_id, Seq(0), 100)
        .await
        .unwrap();
    let seqs: Vec<u64> = events.iter().map(|e| e.seq.0).collect();
    assert_eq!(
        seqs,
        (1..=t.record.last_seq.0).collect::<Vec<_>>(),
        "commit: contiguous seqs, nothing from the stale commit"
    );
    let suffix = store
        .read_events(&s, &rec.run_id, Seq(2), 100)
        .await
        .unwrap();
    assert!(
        suffix.iter().all(|e| e.seq.0 > 2),
        "commit: read_events after"
    );
}

async fn fence_and_invariants(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let rec = create(store, &create_request(&s, "/f")).await;
    let t = lifecycle::apply_acquire(&rec, "w1", T0, 90_000).unwrap();
    commit(store, &t, Fence::None).await.unwrap();
    let rec = t.record;
    let renew = lifecycle::apply_renew(&rec, T0 + 5, 90_000).unwrap();
    let wrong = Fence::Lease(LeaseFence {
        owner: "w1".into(),
        epoch: crate::ids::LeaseEpoch(rec.lease_epoch.0 + 7),
    });
    assert!(
        matches!(
            commit(store, &renew, wrong).await,
            Err(StoreError::LeaseLost { .. })
        ),
        "fence: wrong epoch"
    );
    let intruder = Fence::Lease(LeaseFence {
        owner: "w2".into(),
        epoch: rec.lease_epoch,
    });
    assert!(
        matches!(
            commit(store, &renew, intruder).await,
            Err(StoreError::LeaseLost { .. })
        ),
        "fence: wrong owner"
    );
    let right = Fence::Lease(LeaseFence {
        owner: "w1".into(),
        epoch: rec.lease_epoch,
    });
    commit(store, &renew, right)
        .await
        .expect("fence: holder renews");
    // An invariant violation is refused.
    let mut bad =
        lifecycle::apply_checkpoint(&renew.record, CheckpointReason::Periodic, None, T0 + 6);
    bad.record.depth = 3;
    assert!(
        matches!(
            commit(store, &bad, Fence::None).await,
            Err(StoreError::Invariant(_))
        ),
        "invariants: refused"
    );
}

async fn control_dedup(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let rec = create(store, &create_request(&s, "/d")).await;
    let ct = apply_control(&rec, &stop("c1"), true, T0);
    commit(store, &ct.transition, Fence::None).await.unwrap();
    let (ack, _) = store
        .control_ack(&s, &rec.run_id, "c1")
        .await
        .unwrap()
        .expect("ack stored");
    assert_eq!(ack, ct.ack, "control: ack stored");
    let rec2 = store.load(&s, &rec.run_id).await.unwrap().unwrap();
    let again = apply_control(&rec2, &stop("c1"), true, T0);
    assert!(
        matches!(
            commit(store, &again.transition, Fence::None).await,
            Err(StoreError::ControlDuplicate { .. })
        ),
        "control: duplicate"
    );
    let mut reused = stop("c1");
    reused.kind = ControlKind::Pause;
    let other = apply_control(&rec2, &reused, true, T0);
    assert!(
        matches!(
            commit(store, &other.transition, Fence::None).await,
            Err(StoreError::ControlReused)
        ),
        "control: reused"
    );
}

async fn side_tables(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let rec = create(store, &create_request(&s, "/e")).await;
    let mut t =
        lifecycle::apply_checkpoint(&rec, CheckpointReason::Periodic, Some("one".into()), T0);
    t.idem.push(("k1".into(), Seq(2)));
    t.domain_state = Some((1, json!({ "state": { "x": 1 } })));
    t.results.push((
        crate::events::ResultRef {
            key: "r1".into(),
            bytes: 2,
            content_type: "application/json".into(),
        },
        b"{}".to_vec(),
    ));
    commit(store, &t, Fence::None).await.unwrap();
    assert_eq!(
        store.idem_seen(&s, &rec.run_id, "k1").await.unwrap(),
        Some(Seq(2)),
        "idem"
    );
    assert_eq!(
        store.domain_state(&s, &rec.run_id, 1).await.unwrap(),
        Some(json!({ "state": { "x": 1 } })),
        "domain state"
    );
    assert_eq!(
        store.read_result(&s, &rec.run_id, "r1").await.unwrap(),
        Some(b"{}".to_vec()),
        "result"
    );
    let t2 = lifecycle::apply_checkpoint(
        &t.record,
        CheckpointReason::Periodic,
        Some("two".into()),
        T0,
    );
    commit(store, &t2, Fence::None).await.unwrap();
    let latest = store
        .latest_checkpoint(&s, &rec.run_id)
        .await
        .unwrap()
        .expect("checkpoint");
    assert_eq!(latest.checkpoint_no, 2, "latest checkpoint");
    let mut t3 = lifecycle::apply_checkpoint(&t2.record, CheckpointReason::Periodic, None, T0);
    t3.domain_state = Some((1, json!({ "state": "again" })));
    assert!(
        commit(store, &t3, Fence::None).await.is_err(),
        "domain state is write-once"
    );
}

async fn status_index_and_terminal(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let req = create_request(&s, "/g");
    let rec = create(store, &req).await;
    assert_eq!(
        store.scan_status(&s, RunStatus::Queued, 10).await.unwrap(),
        vec![rec.run_id.clone()],
        "index: queued"
    );
    let t = lifecycle::apply_acquire(&rec, "w1", T0, 90_000).unwrap();
    commit(store, &t, Fence::None).await.unwrap();
    assert!(
        store
            .scan_status(&s, RunStatus::Queued, 10)
            .await
            .unwrap()
            .is_empty(),
        "index: moved off queued"
    );
    assert_eq!(
        store.scan_status(&s, RunStatus::Running, 10).await.unwrap(),
        vec![rec.run_id.clone()],
        "index: running"
    );
    let end = lifecycle::apply_terminal(
        &t.record,
        TerminalStatus::Completed,
        RunOutcome::new("succeeded", None),
        None,
        T0,
    )
    .unwrap();
    commit(store, &end, Fence::None).await.unwrap();
    assert_eq!(
        store
            .scan_status(&s, RunStatus::Completed, 10)
            .await
            .unwrap(),
        vec![rec.run_id.clone()],
        "index: completed"
    );
    let again = store
        .create(
            new_record(&req, RunId::new_v4(), T0),
            first_event(&req),
            None,
        )
        .await
        .unwrap();
    let CreateOutcome::Created { run_id: next, .. } = again else {
        panic!("terminal frees the subject: {again:?}");
    };
    assert_eq!(
        store
            .scan_subject(&s, &req.subject.key().unwrap(), 10)
            .await
            .unwrap(),
        vec![rec.run_id.clone(), next],
        "subject history: every run, oldest first"
    );
}

async fn scopes_are_isolated(store: &dyn AgentRunStore) {
    let a = scope("tenant-a");
    let b = scope("tenant-ab");
    let ra = create(store, &create_request(&a, "/x")).await;
    let rb = create(store, &create_request(&b, "/x")).await;
    assert_eq!(
        store.scan_status(&a, RunStatus::Queued, 10).await.unwrap(),
        vec![ra.run_id.clone()],
        "isolation: a"
    );
    assert_eq!(
        store.scan_status(&b, RunStatus::Queued, 10).await.unwrap(),
        vec![rb.run_id.clone()],
        "isolation: b"
    );
    assert!(
        store.load(&b, &ra.run_id).await.unwrap().is_none(),
        "isolation: load"
    );
}
