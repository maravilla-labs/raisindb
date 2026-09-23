// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The pure half of a control: what a command does to the record.
//!
//! Every command is logged (`ControlReceived`), then either applied
//! (`ControlApplied`) or rejected (`ControlRejected`) — a rejection is still a
//! commit, so the log says who asked for what and why nothing happened.

use serde_json::json;

use crate::control::{payload_digest, ApprovalDecision, ControlAck, ControlCommand, ControlKind};
use crate::events::{CheckpointReason, RunEventKind};
use crate::ids::SteerId;
use crate::record::{AgentRunRecord, PendingKind, SteerEntry, STEER_QUEUE_LIMIT};
use crate::state::{Activity, PauseReason, RunOutcome, RunState, TerminalStatus, WakeReason};
use crate::tx::{Transition, Tx};

/// A control's transition and its ack.
#[derive(Debug, Clone)]
pub struct ControlTransition {
    /// What to commit.
    pub transition: Transition,
    /// The ack (also stored under `ctl\0{control_id}`).
    pub ack: ControlAck,
}

/// Apply `cmd` to `rec`. `authorized` is decided by the caller.
pub fn apply_control(
    rec: &AgentRunRecord,
    cmd: &ControlCommand,
    authorized: bool,
    now: u64,
) -> ControlTransition {
    let mut tx = Tx::new(rec, now);
    tx.push(RunEventKind::ControlReceived {
        control_id: cmd.control_id.clone(),
        command: cmd.kind.clone(),
        issued_by: cmd.issued_by.redacted(),
    });
    let before = tx.clone();
    let result = if !authorized {
        Err("unauthorized".to_owned())
    } else if rec.state.is_terminal() {
        Err("run_terminal".to_owned())
    } else {
        body(&mut tx, cmd)
    };
    let ack = match result {
        Ok(()) => {
            let seq = tx.push(RunEventKind::ControlApplied {
                control_id: cmd.control_id.clone(),
            });
            ControlAck::Applied { seq }
        }
        Err(reason) => {
            tx = before;
            let seq = tx.push(RunEventKind::ControlRejected {
                control_id: cmd.control_id.clone(),
                reason: reason.clone(),
            });
            ControlAck::Rejected { reason, seq }
        }
    };
    tx.control = Some((
        cmd.control_id.clone(),
        ack.clone(),
        payload_digest(&cmd.kind),
    ));
    ControlTransition {
        transition: tx.finish(),
        ack,
    }
}

fn body(tx: &mut Tx, cmd: &ControlCommand) -> Result<(), String> {
    match &cmd.kind {
        ControlKind::Stop { reason } => stop(tx, cmd, reason.clone()),
        ControlKind::Pause => pause(tx),
        ControlKind::Resume {
            budget_increase,
            accept_reducer_change,
        } => resume(tx, budget_increase.as_ref(), *accept_reducer_change),
        ControlKind::Steer { input } => steer(tx, cmd, input.clone()),
        ControlKind::Approve {
            request_id,
            decision,
            subject_digest,
        } => {
            let request = find_open(tx, request_id)?;
            let PendingKind::Approval {
                subject_digest: want,
                ..
            } = &request.kind
            else {
                return Err("request_kind_mismatch".into());
            };
            if want != subject_digest {
                return Err("digest_mismatch".into());
            }
            let (decision, reason) = match decision {
                ApprovalDecision::Approve => ("approve", None),
                ApprovalDecision::Reject { reason } => ("reject", reason.clone()),
            };
            let resolution = json!({
                "kind": "approval", "decision": decision, "reason": reason,
                "subject_digest": subject_digest, "effect_id": request.effect_id,
            });
            resolve(tx, cmd, request_id, resolution);
            Ok(())
        }
        ControlKind::ProvideInput { request_id, value } => {
            let request = find_open(tx, request_id)?;
            if !matches!(request.kind, PendingKind::Input { .. }) {
                return Err("request_kind_mismatch".into());
            }
            let resolution =
                json!({"kind": "input", "value": value, "effect_id": request.effect_id});
            resolve(tx, cmd, request_id, resolution);
            Ok(())
        }
    }
}

fn find_open(tx: &Tx, id: &crate::ids::RequestId) -> Result<crate::record::PendingRequest, String> {
    tx.rec
        .state
        .open_requests()
        .into_iter()
        .find(|r| &r.request_id == id)
        .ok_or_else(|| "request_not_open".to_owned())
}

fn resolve(
    tx: &mut Tx,
    cmd: &ControlCommand,
    id: &crate::ids::RequestId,
    resolution: serde_json::Value,
) {
    tx.push(RunEventKind::RequestResolved {
        request_id: id.clone(),
        resolution,
        by_control: Some(cmd.control_id.clone()),
    });
    tx.remove_request(id);
}

fn stop(tx: &mut Tx, cmd: &ControlCommand, reason: Option<String>) -> Result<(), String> {
    match tx.rec.state.clone() {
        RunState::Cancelling { .. } => Err("already_stopping".into()),
        RunState::Running {
            lease,
            activity: Activity::Operating { op, .. },
        } => {
            let stop = crate::state::StopInfo {
                control_id: cmd.control_id.clone(),
                reason,
            };
            tx.cancel = Some((tx.rec.run_id.clone(), op.op_id.clone()));
            tx.rec.status_reason = Some("user_stop".into());
            tx.set_state(
                RunState::Cancelling { lease, op, stop },
                Some("user_stop".into()),
            );
            Ok(())
        }
        _ => {
            tx.terminate(
                TerminalStatus::Stopped,
                RunOutcome::new("stopped", reason),
                Some("user_stop".into()),
            );
            Ok(())
        }
    }
}

fn pause(tx: &mut Tx) -> Result<(), String> {
    match tx.rec.state.clone() {
        RunState::Running {
            lease,
            activity:
                Activity::Operating {
                    op,
                    pause_requested,
                },
        } => {
            if pause_requested {
                return Err("already_pausing".into());
            }
            tx.rec.state = RunState::Running {
                lease,
                activity: Activity::Operating {
                    op,
                    pause_requested: true,
                },
            };
            Ok(())
        }
        RunState::Running {
            activity: Activity::Idle { open },
            ..
        } => {
            enter_user_pause(tx, open);
            Ok(())
        }
        RunState::Waiting { open } => {
            enter_user_pause(tx, open.into_vec());
            Ok(())
        }
        RunState::Paused { .. } => Err("already_paused".into()),
        RunState::Cancelling { .. } => Err("run_stopping".into()),
        _ => Err("not_pausable".into()),
    }
}

fn enter_user_pause(tx: &mut Tx, open: Vec<crate::record::PendingRequest>) {
    tx.rec.status_reason = Some("user_pause".into());
    tx.set_state(
        RunState::Paused {
            open,
            reason: PauseReason::User,
        },
        Some("user_pause".into()),
    );
    tx.checkpoint(CheckpointReason::Pause, None);
}

fn resume(
    tx: &mut Tx,
    inc: Option<&crate::record::RunBudgets>,
    accept_change: bool,
) -> Result<(), String> {
    let RunState::Paused { open, reason } = tx.rec.state.clone() else {
        return Err("not_paused".into());
    };
    if matches!(&reason, PauseReason::Reducer { reason } if reason == "reducer_changed") {
        if !accept_change {
            return Err("reducer_changed_requires_accept".into());
        }
        if let Some(d) = tx.rec.domain.as_mut() {
            if let Some(hash) = d.pending_artifact_hash.take() {
                d.reducer.artifact_hash = hash;
            }
        }
    }
    if let Some(inc) = inc {
        crate::budget::raise(&mut tx.rec.budgets, inc);
    }
    tx.rec.status_reason = None;
    let from_checkpoint_no = (tx.rec.counters.checkpoint > 0).then_some(tx.rec.counters.checkpoint);
    tx.push(RunEventKind::Resumed { from_checkpoint_no });
    tx.set_state(
        RunState::Queued {
            open,
            wake: WakeReason::Resumed,
        },
        None,
    );
    Ok(())
}

fn steer(tx: &mut Tx, cmd: &ControlCommand, input: serde_json::Value) -> Result<(), String> {
    if matches!(tx.rec.state, RunState::Cancelling { .. }) {
        return Err("run_stopping".into());
    }
    if tx.rec.steer_queue.len() >= STEER_QUEUE_LIMIT {
        return Err("steer_queue_full".into());
    }
    tx.rec.counters.steer += 1;
    let steer_id = SteerId::nth(&tx.rec.run_id, tx.rec.counters.steer);
    let queued_seq = tx.push(RunEventKind::SteerQueued {
        steer_id: steer_id.clone(),
    });
    tx.rec.steer_queue.push(SteerEntry {
        steer_id,
        control_id: cmd.control_id.clone(),
        input,
        queued_seq,
    });
    if let RunState::Waiting { open } = tx.rec.state.clone() {
        tx.set_state(
            RunState::Queued {
                open: open.into_vec(),
                wake: WakeReason::Steer,
            },
            None,
        );
    }
    Ok(())
}
