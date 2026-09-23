// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure lifecycle transitions. No I/O, no clock: `now` is an argument.
//!
//! Each `apply_*` returns a [`Transition`] (record, events, and what to do
//! after the commit) or a refusal. [`TRANSITIONS`] lists the legal status
//! changes as data, so tests can enumerate them.

use serde_json::Value;

use crate::budget::{self, Exceeded};
use crate::events::{CheckpointReason, RunEventKind, TurnCause};
use crate::ids::{CallId, LeaseEpoch, OperationId};
use crate::record::{ActiveOperation, AgentRunRecord, Lease, OperationKind};
use crate::state::{
    Activity, PauseReason, RunOutcome, RunState, RunStatus, TerminalStatus, WakeReason,
};
use crate::tx::{Transition, Tx};

pub use crate::control_apply::{apply_control, ControlTransition};
pub use crate::lifecycle_ops::*;

/// Every legal status change: `(from, to, trigger)`.
pub const TRANSITIONS: &[(RunStatus, RunStatus, &str)] = &[
    (RunStatus::Queued, RunStatus::Running, "acquire_lease"),
    (RunStatus::Queued, RunStatus::Stopped, "control stop"),
    (
        RunStatus::Running,
        RunStatus::Paused,
        "finish_operation with pause requested; control pause at idle; budget under Pause",
    ),
    (
        RunStatus::Running,
        RunStatus::Waiting,
        "domain commit that opens a request with no op; waiting tool outcome",
    ),
    (
        RunStatus::Running,
        RunStatus::Queued,
        "release_lease at a boundary; recovery of an idle expired lease",
    ),
    (
        RunStatus::Running,
        RunStatus::Cancelling,
        "control stop while operating",
    ),
    (
        RunStatus::Running,
        RunStatus::Stopped,
        "control stop at idle",
    ),
    (RunStatus::Running, RunStatus::Completed, "domain complete"),
    (
        RunStatus::Running,
        RunStatus::Failed,
        "domain fail; budget under Fail; reducer contract error",
    ),
    (
        RunStatus::Cancelling,
        RunStatus::Stopped,
        "operation returns; recovery of an expired lease",
    ),
    (
        RunStatus::Waiting,
        RunStatus::Queued,
        "last request resolved or expired; steer; external result",
    ),
    (RunStatus::Waiting, RunStatus::Paused, "control pause"),
    (RunStatus::Waiting, RunStatus::Stopped, "control stop"),
    (RunStatus::Paused, RunStatus::Stopped, "control stop"),
    (RunStatus::Paused, RunStatus::Queued, "control resume"),
];

/// Why a pure transition was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct TransitionRefusal {
    /// Stable code.
    pub code: String,
    /// Detail.
    pub message: String,
}

pub(crate) fn refusal<T>(code: &str, message: impl Into<String>) -> Result<T, TransitionRefusal> {
    Err(TransitionRefusal {
        code: code.into(),
        message: message.into(),
    })
}

/// The caller's claim to the lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseFence {
    /// Holder.
    pub owner: String,
    /// Epoch.
    pub epoch: LeaseEpoch,
}

/// What a commit must prove about the stored record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fence {
    /// Controls: no lease claim.
    None,
    /// Executor commits: the caller must hold the live lease.
    Lease(LeaseFence),
    /// Lease-free domain finalize of a terminal run.
    DomainFinalize {
        /// The state revision the finalizer read.
        expected_state_rev: u64,
    },
}

/// Check `fence` against the STORED record. Returns the refusal code.
pub fn check_fence(
    stored: &AgentRunRecord,
    fence: &Fence,
    now_ms: u64,
) -> Result<(), &'static str> {
    match fence {
        Fence::None => Ok(()),
        Fence::Lease(f) => match stored.state.lease() {
            Some(lease)
                if lease.owner == f.owner
                    && lease.epoch == f.epoch
                    && f.epoch == stored.lease_epoch
                    && now_ms < lease.expires_at_ms =>
            {
                Ok(())
            }
            _ => Err("lease_lost"),
        },
        Fence::DomainFinalize { expected_state_rev } => match &stored.domain {
            Some(d)
                if stored.state.is_terminal()
                    && !d.finalized
                    && d.state_rev == *expected_state_rev =>
            {
                Ok(())
            }
            _ => Err("finalize_fenced"),
        },
    }
}

/// What to start.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OperationSpec {
    /// Kind (default: a tool call).
    pub kind: Option<OperationKind>,
    /// Domain effect.
    pub effect_id: Option<String>,
    /// Replay safety; `None` means the kind's default (model turns and
    /// compaction are safe, everything else is not).
    pub replay_safe: Option<bool>,
    /// Whether a stop may interrupt it (default true).
    pub non_interruptible: bool,
    /// The model call it answers.
    pub for_call_id: Option<CallId>,
    /// For a model turn: the calls its tool results answer.
    pub answers: Vec<CallId>,
    /// What to execute.
    pub input: Option<Value>,
    /// Deadline.
    pub deadline_ms: Option<u64>,
    /// Why a new turn would start.
    pub cause: Option<TurnCause>,
}

/// Why `begin_operation` refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginRefusal {
    /// Not idle under a lease.
    NotIdle,
    /// Requests are open; an operation cannot start.
    OpenRequests,
    /// Rule R12.
    UnansweredCalls(String),
    /// A budget is exceeded.
    Budget(Exceeded),
    /// A child run's objective does not allow this tool.
    ToolNotAllowed(String),
}

/// `Queued → Running{Idle}`.
pub fn apply_acquire(
    rec: &AgentRunRecord,
    owner: &str,
    now: u64,
    ttl_ms: u64,
) -> Result<Transition, TransitionRefusal> {
    let RunState::Queued { open, .. } = rec.state.clone() else {
        return refusal(
            "not_queued",
            format!("run is {}", rec.state.status().as_str()),
        );
    };
    let mut tx = Tx::new(rec, now);
    tx.rec.lease_epoch = tx.rec.lease_epoch.next();
    let epoch = tx.rec.lease_epoch;
    tx.push(RunEventKind::LeaseAcquired {
        owner: owner.into(),
        epoch,
    });
    let lease = Lease {
        owner: owner.into(),
        epoch,
        expires_at_ms: now + ttl_ms,
    };
    tx.set_state(
        RunState::Running {
            lease,
            activity: Activity::Idle { open },
        },
        None,
    );
    Ok(tx.finish())
}

/// Extend the lease. No events.
pub fn apply_renew(
    rec: &AgentRunRecord,
    now: u64,
    ttl_ms: u64,
) -> Result<Transition, TransitionRefusal> {
    let mut tx = Tx::new(rec, now);
    match &mut tx.rec.state {
        RunState::Running { lease, .. } | RunState::Cancelling { lease, .. } => {
            lease.expires_at_ms = now + ttl_ms
        }
        _ => return refusal("no_lease", "nothing to renew"),
    }
    Ok(tx.finish())
}

/// `Running{Idle} → Queued` at a boundary.
pub fn apply_release(
    rec: &AgentRunRecord,
    now: u64,
    wake: bool,
) -> Result<Transition, TransitionRefusal> {
    let RunState::Running {
        activity: Activity::Idle { open },
        ..
    } = rec.state.clone()
    else {
        return refusal("not_idle", "release is legal only between operations");
    };
    let mut tx = Tx::new(rec, now);
    tx.set_state(
        RunState::Queued {
            open,
            wake: WakeReason::LeaseReleased,
        },
        None,
    );
    if !wake {
        tx.wake = None;
    }
    Ok(tx.finish())
}

/// `Running{Idle} → Running{Operating}`.
pub fn apply_begin(
    rec: &AgentRunRecord,
    spec: OperationSpec,
    now: u64,
) -> Result<Transition, BeginRefusal> {
    let mut tx = Tx::new(rec, now);
    begin_in(&mut tx, spec)?;
    Ok(tx.finish())
}

/// Start an operation inside a transition under construction. Leaves `tx`
/// untouched on refusal.
pub(crate) fn begin_in(tx: &mut Tx, spec: OperationSpec) -> Result<ActiveOperation, BeginRefusal> {
    let RunState::Running {
        lease,
        activity: Activity::Idle { open },
    } = tx.rec.state.clone()
    else {
        return Err(BeginRefusal::NotIdle);
    };
    if !open.is_empty() {
        return Err(BeginRefusal::OpenRequests);
    }
    let kind = spec.kind.clone().unwrap_or(OperationKind::ToolCall);
    if let Some(tool) = denied_tool(&tx.rec, &kind, spec.input.as_ref()) {
        return Err(BeginRefusal::ToolNotAllowed(tool));
    }
    let starts_turn = tx.rec.current_turn.is_none();
    if let Some(e) = budget::check_before_op(&tx.rec, &kind, starts_turn, tx.now_ms) {
        return Err(BeginRefusal::Budget(e));
    }
    match &kind {
        OperationKind::ModelTurn => {
            if let Some(missing) = tx
                .rec
                .unanswered_calls
                .iter()
                .find(|c| !spec.answers.contains(c))
            {
                return Err(BeginRefusal::UnansweredCalls(format!(
                    "call '{missing}' is unanswered"
                )));
            }
            tx.rec.unanswered_calls.clear();
        }
        _ => {
            if let Some(call) = &spec.for_call_id {
                if !tx.rec.unanswered_calls.contains(call) {
                    return Err(BeginRefusal::UnansweredCalls(format!(
                        "call '{call}' is not unanswered"
                    )));
                }
            }
        }
    }
    if starts_turn {
        start_turn(tx, spec.cause.clone());
    }
    tx.rec.counters.op += 1;
    let op_id = OperationId::nth(&tx.rec.run_id, tx.rec.counters.op);
    let replay_safe = spec.replay_safe.unwrap_or(matches!(
        kind,
        OperationKind::ModelTurn | OperationKind::Compaction
    ));
    let op = ActiveOperation {
        op_id: op_id.clone(),
        kind,
        effect_id: spec.effect_id.clone(),
        idempotency_key: op_id.0.clone(),
        replay_safe,
        interruptible: !spec.non_interruptible,
        for_call_id: spec.for_call_id.clone(),
        input: spec.input,
        started_at_ms: tx.now_ms,
        deadline_ms: spec.deadline_ms,
        lease_epoch: tx.rec.lease_epoch,
        attempt: 1,
    };
    tx.push(started_event(&op));
    tx.rec.state = RunState::Running {
        lease,
        activity: Activity::Operating {
            op: op.clone(),
            pause_requested: false,
        },
    };
    Ok(op)
}

/// The tool a child's objective does not allow, if `kind`/`input` name one.
pub(crate) fn denied_tool(
    rec: &AgentRunRecord,
    kind: &OperationKind,
    input: Option<&Value>,
) -> Option<String> {
    let allowed = &rec.delegation.as_ref()?.objective.allowed_tools;
    if *kind != OperationKind::ToolCall || allowed.is_empty() {
        return None;
    }
    let tool = input
        .and_then(|i| i.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("");
    (!crate::child::tool_allowed(allowed, tool)).then(|| tool.to_string())
}

pub(crate) fn start_turn(tx: &mut Tx, cause: Option<TurnCause>) {
    tx.rec.counters.turn += 1;
    let turn = crate::ids::TurnNo(tx.rec.counters.turn);
    tx.rec.current_turn = Some(turn);
    tx.rec.usage.turns = tx.rec.counters.turn;
    let cause = cause.unwrap_or(if turn.0 == 1 {
        TurnCause::Input
    } else {
        TurnCause::Continuation
    });
    tx.push(RunEventKind::TurnStarted { turn, cause });
}

pub(crate) fn started_event(op: &ActiveOperation) -> RunEventKind {
    RunEventKind::OperationStarted {
        op_id: op.op_id.clone(),
        kind: op.kind.clone(),
        effect_id: op.effect_id.clone(),
        idempotency_key: op.idempotency_key.clone(),
        replay_safe: op.replay_safe,
        interruptible: op.interruptible,
        for_call_id: op.for_call_id.clone(),
        attempt: op.attempt,
    }
}

/// Log `BudgetExceeded` and apply the policy (pause with a checkpoint, or fail).
pub(crate) fn exceed(tx: &mut Tx, e: &Exceeded) {
    tx.push(RunEventKind::BudgetExceeded {
        which: e.which.clone(),
        limit: e.limit,
        used: e.used,
    });
    let reason = format!("budget_exceeded:{}", e.which);
    match tx.rec.budgets.on_exceeded {
        crate::record::BudgetPolicy::Pause => {
            let open = tx.rec.state.open_requests();
            tx.rec.status_reason = Some(reason.clone());
            tx.set_state(
                RunState::Paused {
                    open,
                    reason: PauseReason::Budget {
                        which: e.which.clone(),
                    },
                },
                Some(reason),
            );
            tx.checkpoint(CheckpointReason::Pause, None);
        }
        crate::record::BudgetPolicy::Fail => {
            let outcome = RunOutcome {
                kind: "failed".into(),
                code: Some(reason.clone()),
                ..RunOutcome::default()
            };
            tx.terminate(TerminalStatus::Failed, outcome, Some(reason));
        }
    }
}

/// A run at idle exceeded a budget: pause or fail per policy.
pub fn apply_budget_exceeded(rec: &AgentRunRecord, e: &Exceeded, now: u64) -> Transition {
    let mut tx = Tx::new(rec, now);
    exceed(&mut tx, e);
    tx.finish()
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
