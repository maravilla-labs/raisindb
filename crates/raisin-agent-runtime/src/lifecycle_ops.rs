// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure transitions after an operation starts: finishing, steering, waiting,
//! ending, expiry, takeover. Re-exported from [`crate::lifecycle`].

use serde_json::Value;

use crate::budget;
use crate::events::{CheckpointReason, OpOutcome, OpUsage, ResultRef, RunEventKind, TurnCause};
use crate::ids::{CallId, OperationId, RequestId};
use crate::lifecycle::{refusal, start_turn, started_event, LeaseFence, TransitionRefusal};
use crate::record::{
    ActiveOperation, AgentRunRecord, Lease, OperationKind, PendingKind, PendingRequest,
};
use crate::state::{
    Activity, NonEmpty, PauseReason, RunOutcome, RunState, TerminalStatus, WakeReason,
};
use crate::tx::{Transition, Tx};

/// What an executor reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OperationResult {
    /// Outcome.
    pub outcome: Option<OpOutcome>,
    /// Token usage.
    pub usage: Option<OpUsage>,
    /// Result payload (JSON bytes), stored under `res\0`.
    pub payload: Option<Value>,
    /// Model turn only: the tool-call ids the model asked for.
    pub tool_calls: Vec<CallId>,
    /// For `Waiting`: the key a delivery will name.
    pub resume_key: Option<String>,
}

/// A result at least this big is "large": checkpoints reference it.
pub const LARGE_RESULT_BYTES: u64 = 4 * 1024;
/// How many large-result refs the record keeps (a rolling window).
pub const LARGE_RESULT_WINDOW: usize = 32;

fn result_ref(tx: &mut Tx, op: &ActiveOperation, payload: &Option<Value>) -> Option<ResultRef> {
    let payload = payload.as_ref()?;
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    let r = ResultRef {
        key: format!("op:{}:{}", op.op_id, op.attempt),
        bytes: bytes.len() as u64,
        content_type: "application/json".into(),
    };
    tx.results.push((r.clone(), bytes));
    if r.bytes >= LARGE_RESULT_BYTES {
        let window = &mut tx.rec.large_results;
        window.push(r.clone());
        if window.len() > LARGE_RESULT_WINDOW {
            let excess = window.len() - LARGE_RESULT_WINDOW;
            window.drain(..excess);
        }
    }
    Some(r)
}

pub(crate) fn completed(
    tx: &mut Tx,
    op: &ActiveOperation,
    result: &OperationResult,
    outcome: OpOutcome,
) {
    let result_ref = result_ref(tx, op, &result.payload);
    let seq = tx.push(RunEventKind::OperationCompleted {
        op_id: op.op_id.clone(),
        kind: Some(op.kind.clone()),
        effect_id: op.effect_id.clone(),
        for_call_id: op.for_call_id.clone(),
        tool: op.tool(),
        outcome,
        usage: result.usage,
        result_ref,
        tool_calls: result.tool_calls.clone(),
    });
    tx.idem.push((op.idempotency_key.clone(), seq));
    budget::accumulate(&mut tx.rec.usage, &op.kind, outcome, result.usage);
    if op.kind == OperationKind::ModelTurn {
        tx.rec.unanswered_calls = result.tool_calls.clone();
    }
}

/// `Running{Operating}` or `Cancelling` → whatever the result implies.
pub fn apply_finish(
    rec: &AgentRunRecord,
    op_id: &OperationId,
    result: OperationResult,
    now: u64,
) -> Result<Transition, TransitionRefusal> {
    let outcome = result.outcome.unwrap_or(OpOutcome::Succeeded);
    let mut tx = Tx::new(rec, now);
    match rec.state.clone() {
        RunState::Running {
            lease,
            activity:
                Activity::Operating {
                    op,
                    pause_requested,
                },
        } if &op.op_id == op_id => {
            if outcome == OpOutcome::Waiting
                && result.resume_key.as_deref().is_none_or(str::is_empty)
            {
                return refusal(
                    "waiting_without_resume_key",
                    "a waiting result must name a resume_key",
                );
            }
            completed(&mut tx, &op, &result, outcome);
            let mut open = Vec::new();
            if outcome == OpOutcome::Waiting {
                tx.rec.counters.request += 1;
                let request = PendingRequest {
                    request_id: RequestId::nth(&rec.run_id, tx.rec.counters.request),
                    kind: PendingKind::External {
                        op_id: op.op_id.clone(),
                        resume_key: result.resume_key.clone().unwrap_or_default(),
                        for_call_id: op.for_call_id.clone(),
                        tool: op.tool(),
                    },
                    requested_by: Some(op.op_id.clone()),
                    effect_id: op.effect_id.clone(),
                    created_seq: tx.next_seq(),
                    expires_at_ms: None,
                };
                tx.push(RunEventKind::RequestOpened {
                    request: request.clone(),
                });
                open.push(request);
            }
            if pause_requested {
                tx.set_state(
                    RunState::Paused {
                        open,
                        reason: PauseReason::User,
                    },
                    Some("user_pause".into()),
                );
                tx.checkpoint(CheckpointReason::Pause, None);
            } else if let Some(open) = NonEmpty::from_vec(open) {
                tx.set_state(RunState::Waiting { open }, None);
            } else {
                tx.rec.state = RunState::Running {
                    lease,
                    activity: Activity::Idle { open: Vec::new() },
                };
            }
            // A tool that waits for a child whose hand-back ALREADY landed is
            // answered from the mailbox at once: the child finished first.
            if outcome == OpOutcome::Waiting {
                let key = result.resume_key.clone().unwrap_or_default();
                let landed = tx
                    .rec
                    .children
                    .iter()
                    .find(|l| l.delivered && l.resume_key() == key)
                    .cloned();
                if let Some(link) = landed {
                    if let Some(rk) = &link.result_key {
                        crate::child_apply::resolve_child_waits(
                            &mut tx,
                            &link.run_id,
                            rk,
                            link.status,
                        );
                    }
                }
            }
        }
        RunState::Cancelling { op, stop, .. } if &op.op_id == op_id => {
            if outcome == OpOutcome::Cancelled {
                tx.push(RunEventKind::OperationCancelled {
                    op_id: op.op_id.clone(),
                    acknowledged: true,
                    effect_id: op.effect_id.clone(),
                });
            } else {
                completed(&mut tx, &op, &result, outcome);
                tx.push(RunEventKind::OperationCancelled {
                    op_id: op.op_id.clone(),
                    acknowledged: false,
                    effect_id: op.effect_id.clone(),
                });
            }
            let outcome = RunOutcome::new("stopped", stop.reason.clone());
            tx.terminate(TerminalStatus::Stopped, outcome, Some("user_stop".into()));
        }
        _ => {
            return refusal(
                "op_not_active",
                format!("operation {op_id} is not in flight"),
            )
        }
    }
    Ok(tx.finish())
}

/// Consume every queued steer at the idle boundary. `None` when none queued.
pub fn apply_consume_steers(rec: &AgentRunRecord, now: u64) -> Option<Transition> {
    if rec.steer_queue.is_empty()
        || !matches!(
            rec.state,
            RunState::Running {
                activity: Activity::Idle { .. },
                ..
            }
        )
    {
        return None;
    }
    let mut tx = Tx::new(rec, now);
    for steer in std::mem::take(&mut tx.rec.steer_queue) {
        if tx.rec.current_turn.is_none() {
            start_turn(
                &mut tx,
                Some(TurnCause::Steer {
                    steer_id: steer.steer_id.clone(),
                }),
            );
        }
        let turn = tx.rec.current_turn.expect("turn started above");
        tx.push(RunEventKind::SteerConsumed {
            steer_id: steer.steer_id,
            turn,
            input: steer.input,
        });
    }
    Some(tx.finish())
}

/// A request a planner opens at idle.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRequest {
    /// What.
    pub kind: PendingKind,
    /// The effect that asked.
    pub effect_id: Option<String>,
    /// Expiry.
    pub expires_at_ms: Option<u64>,
}

pub(crate) fn open_request(tx: &mut Tx, req: NewRequest) -> PendingRequest {
    tx.rec.counters.request += 1;
    let request = PendingRequest {
        request_id: RequestId::nth(&tx.rec.run_id, tx.rec.counters.request),
        kind: req.kind,
        requested_by: None,
        effect_id: req.effect_id,
        created_seq: tx.next_seq(),
        expires_at_ms: req.expires_at_ms,
    };
    tx.push(RunEventKind::RequestOpened {
        request: request.clone(),
    });
    request
}

/// `Running{Idle} → Waiting` with new requests.
pub fn apply_wait(
    rec: &AgentRunRecord,
    requests: Vec<NewRequest>,
    now: u64,
) -> Result<Transition, TransitionRefusal> {
    let RunState::Running {
        activity: Activity::Idle { mut open },
        ..
    } = rec.state.clone()
    else {
        return refusal("not_idle", "requests open only between operations");
    };
    let mut tx = Tx::new(rec, now);
    for req in requests {
        if let PendingKind::Child { child_run_id } = &req.kind {
            match rec.children.iter().find(|l| &l.run_id == child_run_id) {
                None => {
                    return refusal(
                        "unknown_child",
                        format!("{child_run_id} is not a child of this run"),
                    )
                }
                Some(l) if l.delivered => {
                    return refusal(
                        "child_already_completed",
                        "the child's hand-back is already in the mailbox",
                    )
                }
                Some(_) => {}
            }
        }
        open.push(open_request(&mut tx, req));
    }
    let Some(open) = NonEmpty::from_vec(open) else {
        return refusal("nothing_to_wait_for", "no request to wait on");
    };
    tx.set_state(RunState::Waiting { open }, None);
    Ok(tx.finish())
}

/// `Running{Idle} → Terminal`.
pub fn apply_terminal(
    rec: &AgentRunRecord,
    status: TerminalStatus,
    outcome: RunOutcome,
    reason: Option<String>,
    now: u64,
) -> Result<Transition, TransitionRefusal> {
    if !matches!(
        rec.state,
        RunState::Running {
            activity: Activity::Idle { .. },
            ..
        }
    ) {
        return refusal("not_idle", "a run ends from its idle boundary");
    }
    let mut tx = Tx::new(rec, now);
    tx.terminate(status, outcome, reason);
    Ok(tx.finish())
}

/// A checkpoint at idle.
pub fn apply_checkpoint(
    rec: &AgentRunRecord,
    reason: CheckpointReason,
    summary: Option<String>,
    now: u64,
) -> Transition {
    let mut tx = Tx::new(rec, now);
    tx.checkpoint(reason, summary);
    tx.finish()
}

/// Close every open request past its expiry. `None` when nothing expired.
pub fn apply_expire_requests(rec: &AgentRunRecord, now: u64) -> Option<Transition> {
    let expired: Vec<RequestId> = rec
        .state
        .open_requests()
        .into_iter()
        .filter(|r| r.expires_at_ms.is_some_and(|at| at <= now))
        .map(|r| r.request_id)
        .collect();
    if expired.is_empty() {
        return None;
    }
    let mut tx = Tx::new(rec, now);
    for id in expired {
        tx.push(RunEventKind::RequestClosed {
            request_id: id.clone(),
            reason: "expired".into(),
        });
        tx.remove_request(&id);
    }
    Some(tx.finish())
}

/// Result of a takeover.
#[derive(Debug, Clone)]
pub struct Takeover {
    /// The transition.
    pub transition: Transition,
    /// Set when the new owner must re-dispatch the in-flight operation.
    pub redispatch: Option<LeaseFence>,
}

/// Take over an EXPIRED lease (the caller checks expiry).
pub fn apply_takeover(
    rec: &AgentRunRecord,
    owner: &str,
    now: u64,
    ttl_ms: u64,
) -> Result<Takeover, TransitionRefusal> {
    let Some(old) = rec.state.lease().cloned() else {
        return refusal("no_lease", "nothing to take over");
    };
    let mut tx = Tx::new(rec, now);
    let taken_over = |tx: &mut Tx| {
        tx.rec.lease_epoch = tx.rec.lease_epoch.next();
        let epoch = tx.rec.lease_epoch;
        tx.push(RunEventKind::LeaseTakenOver {
            from_owner: old.owner.clone(),
            from_epoch: old.epoch,
            owner: owner.into(),
            epoch,
        });
        Lease {
            owner: owner.into(),
            epoch,
            expires_at_ms: now + ttl_ms,
        }
    };
    let mut redispatch = None;
    match rec.state.clone() {
        RunState::Cancelling { op, stop, .. } => {
            let lease = taken_over(&mut tx);
            tx.rec.state = RunState::Cancelling {
                lease,
                op: op.clone(),
                stop: stop.clone(),
            };
            tx.push(RunEventKind::OperationCancelled {
                op_id: op.op_id.clone(),
                acknowledged: false,
                effect_id: op.effect_id.clone(),
            });
            tx.terminate(
                TerminalStatus::Stopped,
                RunOutcome::new("stopped", stop.reason),
                Some("user_stop".into()),
            );
        }
        RunState::Running {
            activity:
                Activity::Operating {
                    mut op,
                    pause_requested,
                },
            ..
        } if op.replay_safe => {
            let lease = taken_over(&mut tx);
            op.attempt += 1;
            op.lease_epoch = lease.epoch;
            op.started_at_ms = now;
            tx.push(started_event(&op));
            redispatch = Some(LeaseFence {
                owner: owner.into(),
                epoch: lease.epoch,
            });
            tx.rec.state = RunState::Running {
                lease,
                activity: Activity::Operating {
                    op,
                    pause_requested,
                },
            };
        }
        RunState::Running {
            activity: Activity::Operating { op, .. },
            ..
        } => {
            let lease = taken_over(&mut tx);
            tx.rec.state = RunState::Running {
                lease,
                activity: Activity::Idle { open: Vec::new() },
            };
            tx.push(RunEventKind::OperationAbandoned {
                op_id: op.op_id.clone(),
                reason: "lease_expired_not_replay_safe".into(),
                effect_id: op.effect_id.clone(),
                for_call_id: op.for_call_id.clone(),
            });
            tx.rec.usage.consecutive_op_failures += 1;
            tx.set_state(
                RunState::Queued {
                    open: Vec::new(),
                    wake: WakeReason::LeaseReleased,
                },
                Some("lease_expired".into()),
            );
        }
        RunState::Running {
            activity: Activity::Idle { open },
            ..
        } => {
            tx.set_state(
                RunState::Queued {
                    open,
                    wake: WakeReason::LeaseReleased,
                },
                Some("lease_expired".into()),
            );
        }
        _ => return refusal("no_lease", "nothing to take over"),
    }
    Ok(Takeover {
        transition: tx.finish(),
        redispatch,
    })
}

/// Pause at idle because the domain reducer cannot be used.
pub fn apply_reducer_pause(
    rec: &AgentRunRecord,
    reason: &str,
    now: u64,
) -> Result<Transition, TransitionRefusal> {
    let RunState::Running {
        activity: Activity::Idle { open },
        ..
    } = rec.state.clone()
    else {
        return refusal("not_idle", "reducer pause happens at the idle boundary");
    };
    let mut tx = Tx::new(rec, now);
    tx.rec.status_reason = Some(reason.into());
    tx.set_state(
        RunState::Paused {
            open,
            reason: PauseReason::Reducer {
                reason: reason.into(),
            },
        },
        Some(reason.into()),
    );
    tx.checkpoint(CheckpointReason::Pause, None);
    Ok(tx.finish())
}
