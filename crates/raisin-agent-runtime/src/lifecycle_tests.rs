// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Unit tests of the pure transitions.

use serde_json::json;

use crate::control::{ActorRef, ApprovalDecision, ControlAck, ControlCommand, ControlKind};
use crate::events::{OpOutcome, RunEventKind};
use crate::ids::{ControlId, RequestId, RunId, Seq};
use crate::lifecycle::*;
use crate::record::{AgentRunRecord, PendingKind};
use crate::service::new_record;
use crate::state::{Activity, RunOutcome, RunState, RunStatus, TerminalStatus};

const NOW: u64 = 1_000;

fn queued() -> AgentRunRecord {
    let req = crate::conformance::create_request(&crate::conformance::scope("t"), "/a");
    new_record(&req, RunId("r".into()), NOW)
}

fn idle() -> AgentRunRecord {
    apply_acquire(&queued(), "w", NOW, 90_000).unwrap().record
}

fn cmd(id: &str, kind: ControlKind) -> ControlCommand {
    ControlCommand {
        control_id: ControlId(id.into()),
        kind,
        issued_by: ActorRef::user("alice"),
        at_ms: NOW,
    }
}

fn waiting_on_approval() -> AgentRunRecord {
    let req = NewRequest {
        kind: PendingKind::Approval {
            subject_digest: "d1".into(),
            digest_alg: "sha256".into(),
            summary: "s".into(),
            scope: None,
            changes: None,
        },
        effect_id: Some("1:0".into()),
        expires_at_ms: Some(NOW + 10),
    };
    apply_wait(&idle(), vec![req], NOW).unwrap().record
}

#[test]
fn acquire_only_from_queued() {
    assert!(apply_acquire(&idle(), "w2", NOW, 1).is_err());
    let t = apply_acquire(&queued(), "w", NOW, 90_000).unwrap();
    assert_eq!(t.record.lease_epoch.0, 1);
    assert_eq!(t.record.last_seq, Seq(1 + t.events.len() as u64));
}

#[test]
fn begin_refuses_while_requests_are_open() {
    let mut rec = idle();
    let open = waiting_on_approval().state.open_requests();
    if let RunState::Running { activity, .. } = &mut rec.state {
        *activity = Activity::Idle { open };
    }
    assert_eq!(
        apply_begin(&rec, OperationSpec::default(), NOW).unwrap_err(),
        BeginRefusal::OpenRequests
    );
}

#[test]
fn waiting_result_needs_a_resume_key() {
    let rec = apply_begin(&idle(), OperationSpec::default(), NOW)
        .unwrap()
        .record;
    let op = rec.state.active_op().unwrap().op_id.clone();
    let r = OperationResult {
        outcome: Some(OpOutcome::Waiting),
        ..OperationResult::default()
    };
    assert_eq!(
        apply_finish(&rec, &op, r, NOW).unwrap_err().code,
        "waiting_without_resume_key"
    );
}

#[test]
fn approve_requires_the_matching_digest() {
    let rec = waiting_on_approval();
    let id = RequestId("r/req/1".into());
    let wrong = cmd(
        "a1",
        ControlKind::Approve {
            request_id: id.clone(),
            decision: ApprovalDecision::Approve,
            subject_digest: "other".into(),
        },
    );
    let ct = apply_control(&rec, &wrong, true, NOW);
    assert!(
        matches!(ct.ack, ControlAck::Rejected { ref reason, .. } if reason == "digest_mismatch")
    );
    assert_eq!(
        ct.transition.record.state.status(),
        RunStatus::Waiting,
        "a rejection changes nothing"
    );
    let right = cmd(
        "a2",
        ControlKind::Approve {
            request_id: id,
            decision: ApprovalDecision::Approve,
            subject_digest: "d1".into(),
        },
    );
    let ct = apply_control(&rec, &right, true, NOW);
    assert!(matches!(ct.ack, ControlAck::Applied { .. }));
    assert_eq!(ct.transition.record.state.status(), RunStatus::Queued);
    let resolved = ct.transition.events.iter().find_map(|e| match &e.kind {
        RunEventKind::RequestResolved { resolution, .. } => Some(resolution.clone()),
        _ => None,
    });
    assert_eq!(resolved.unwrap()["effect_id"], json!("1:0"));
}

#[test]
fn provide_input_to_an_approval_is_a_kind_mismatch() {
    let rec = waiting_on_approval();
    let c = cmd(
        "i",
        ControlKind::ProvideInput {
            request_id: RequestId("r/req/1".into()),
            value: json!(1),
        },
    );
    assert!(
        matches!(apply_control(&rec, &c, true, NOW).ack, ControlAck::Rejected { ref reason, .. } if reason == "request_kind_mismatch")
    );
}

#[test]
fn pause_while_operating_only_marks_the_op() {
    let rec = apply_begin(&idle(), OperationSpec::default(), NOW)
        .unwrap()
        .record;
    let t = apply_control(&rec, &cmd("p", ControlKind::Pause), true, NOW).transition;
    assert!(matches!(
        t.record.state,
        RunState::Running {
            activity: Activity::Operating {
                pause_requested: true,
                ..
            },
            ..
        }
    ));
    assert_eq!(
        t.record.lease_epoch, rec.lease_epoch,
        "the lease is kept until the op returns"
    );
}

#[test]
fn stop_at_idle_ends_the_run_in_one_commit() {
    let t = apply_control(
        &idle(),
        &cmd("s", ControlKind::Stop { reason: None }),
        true,
        NOW,
    )
    .transition;
    assert!(matches!(
        t.record.state,
        RunState::Terminal {
            terminal: TerminalStatus::Stopped,
            ..
        }
    ));
    assert!(t.record.state.lease().is_none());
}

#[test]
fn expired_requests_close_and_requeue() {
    let rec = waiting_on_approval();
    assert!(apply_expire_requests(&rec, NOW + 5).is_none());
    let t = apply_expire_requests(&rec, NOW + 10).unwrap();
    assert_eq!(t.record.state.status(), RunStatus::Queued);
    assert!(t.wake.is_some());
}

#[test]
fn checkpoint_seq_is_the_seq_of_its_event() {
    let t = apply_control(&idle(), &cmd("p", ControlKind::Pause), true, NOW).transition;
    let ckpt = t.checkpoint.clone().unwrap();
    let base = idle().last_seq.0;
    let pos = t
        .events
        .iter()
        .position(|e| matches!(e.kind, RunEventKind::CheckpointWritten { .. }))
        .unwrap();
    assert_eq!(ckpt.at_seq, Seq(base + 1 + pos as u64));
    assert_eq!(t.record.last_checkpoint_seq, Some(ckpt.at_seq));
}

#[test]
fn transition_table_names_every_non_terminal_status() {
    for s in [
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::Waiting,
        RunStatus::Paused,
        RunStatus::Cancelling,
    ] {
        assert!(
            TRANSITIONS.iter().any(|(f, _, _)| *f == s),
            "{s:?} has no way out"
        );
    }
    for s in [RunStatus::Completed, RunStatus::Failed, RunStatus::Stopped] {
        assert!(
            !TRANSITIONS.iter().any(|(f, _, _)| *f == s),
            "{s:?} is terminal"
        );
    }
}

#[test]
fn terminal_clears_everything_a_terminal_record_may_not_hold() {
    let mut rec = idle();
    rec.unanswered_calls = vec![crate::ids::CallId("c".into())];
    let t = apply_terminal(
        &rec,
        TerminalStatus::Completed,
        RunOutcome::default(),
        None,
        NOW,
    )
    .unwrap();
    assert!(t.record.unanswered_calls.is_empty());
    crate::record::check_invariants(Some(&rec), &t.record).unwrap();
}
