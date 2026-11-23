// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The durable event log.
//!
//! Only durable facts are logged: lifecycle, control, operation, wait,
//! checkpoint and usage. Token and text streaming stays on the ephemeral
//! conversation broadcaster. The log is also the domain reducer's inbox: the
//! reducer is fed the events after `domain.delivered_seq`.
//!
//! Operation events are SELF-DESCRIBING (they repeat the operation's kind,
//! effect and call), so turning one into a reducer event needs no join against
//! older events.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::control::{ActorRef, ControlKind};
use crate::ids::{
    CallId, ControlId, LeaseEpoch, OperationId, Principal, RequestId, RunId, Seq, SteerId,
    SubjectRef, TurnNo,
};
use crate::record::{OperationKind, PendingRequest, RunBudgets};
use crate::state::{RunOutcome, RunStatus, TerminalStatus};

/// One persisted event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    /// Run.
    pub run_id: RunId,
    /// Contiguous, assigned by the store inside the commit.
    pub seq: Seq,
    /// Commit time, epoch ms.
    pub at_ms: u64,
    /// Open turn, if any.
    pub turn: Option<TurnNo>,
    /// Operation, if the event is about one.
    pub op_id: Option<OperationId>,
    /// What happened.
    pub kind: RunEventKind,
}

/// An event as a transition emits it; the store assigns `seq` and `at_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEvent {
    /// Open turn, if any.
    pub turn: Option<TurnNo>,
    /// Operation, if the event is about one.
    pub op_id: Option<OperationId>,
    /// What happened.
    pub kind: RunEventKind,
}

/// Why a turn started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum TurnCause {
    /// The run's input.
    Input,
    /// A steer.
    Steer {
        /// Which.
        steer_id: SteerId,
    },
    /// A resolved request.
    Request {
        /// Which.
        request_id: RequestId,
    },
    /// A resume.
    Resume,
    /// Continuing after a boundary.
    Continuation,
}

/// Outcome of an operation; mirrors the tool-result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpOutcome {
    /// Done.
    Succeeded,
    /// Parked on something external (opens an external request).
    Waiting,
    /// Failed, retryable.
    Retryable,
    /// Blocked.
    Blocked,
    /// Failed.
    Failed,
    /// Cancelled by a stop.
    Cancelled,
}

impl OpOutcome {
    /// Failed, retryable or blocked.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Retryable | Self::Blocked)
    }
}

/// Token usage of one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpUsage {
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
}

/// Why an operation did not succeed, safe to show a person.
///
/// The executor's own message carries provider internals ("AI fallback
/// failed: API request failed: HTTP 504 Gateway Timeout: class=timeout");
/// that stays in the stored result. The event carries only a class and a
/// fixed sentence for it, so a client can say "the AI service timed out,
/// retrying (attempt 2)" instead of showing "Thinking…" while a turn fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpFailure {
    /// `timeout`, `rate_limit`, `overloaded`, `upstream`, `unauthorized`,
    /// `config`, `tool_error`, … or `error` when nothing better is known.
    pub class: String,
    /// A short sentence for that class; never the executor's own text.
    pub message: String,
}

impl OpFailure {
    /// The failure an operation result describes (its payload's
    /// `error_class` and `message`), or `None` for a success.
    pub fn from_result(outcome: OpOutcome, payload: Option<&serde_json::Value>) -> Option<Self> {
        if !matches!(
            outcome,
            OpOutcome::Retryable | OpOutcome::Failed | OpOutcome::Blocked
        ) {
            return None;
        }
        let declared = payload
            .and_then(|p| p.get("error_class"))
            .and_then(|c| c.as_str())
            .unwrap_or("error")
            .to_ascii_lowercase();
        let raw = payload
            .and_then(|p| p.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let class = refine_class(&declared, &raw);
        let message = sentence_for(&class).to_string();
        Some(Self { class, message })
    }
}

/// A generic class ("provider", "error") refined from what the raw message
/// says. Only the class is taken from it, never the text.
fn refine_class(declared: &str, raw: &str) -> String {
    let generic = matches!(declared, "provider" | "error" | "unknown" | "");
    if !generic {
        return match declared {
            "rate_limited" | "ratelimit" | "too_many_requests" => "rate_limit".into(),
            other => other.into(),
        };
    }
    if let Some(i) = raw.find("class=") {
        let tail = &raw[i + 6..];
        let c: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !c.is_empty() && c != "provider" {
            return refine_class(&c, "");
        }
    }
    if raw.contains("timed out") || raw.contains("timeout") || raw.contains(" 504") {
        "timeout".into()
    } else if raw.contains(" 429") || raw.contains("rate limit") {
        "rate_limit".into()
    } else if raw.contains(" 529") || raw.contains("overloaded") || raw.contains(" 503") {
        "overloaded".into()
    } else if raw.contains(" 500") || raw.contains(" 502") || raw.contains("bad gateway") {
        "upstream".into()
    } else if declared == "provider" {
        "upstream".into()
    } else {
        "error".into()
    }
}

fn sentence_for(class: &str) -> &'static str {
    match class {
        "timeout" => "The AI service did not answer in time.",
        "rate_limit" => "The AI service is limiting requests right now.",
        "overloaded" => "The AI service is overloaded right now.",
        "upstream" => "The AI service returned an error.",
        "unauthorized" => "This operation is not permitted.",
        "config" => "The agent is not configured correctly.",
        "tool_error" => "A tool failed.",
        "budget" | "budget_exceeded" => "The run reached its budget.",
        _ => "The operation failed.",
    }
}

/// A large payload stored beside the run, never inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultRef {
    /// Key under `res\0`.
    pub key: String,
    /// Size.
    pub bytes: u64,
    /// Media type.
    pub content_type: String,
}

/// Why a checkpoint was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    /// Context compaction.
    Compaction,
    /// Entering pause.
    Pause,
    /// Periodic.
    Periodic,
    /// Before a terminal state.
    BeforeTerminal,
    /// The domain asked.
    DomainRequested,
    /// A child was spawned with a snapshot of this run.
    Delegation,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum RunEventKind {
    RunCreated {
        subject: SubjectRef,
        principal: Principal,
        budgets: RunBudgets,
        input: Value,
    },
    StatusChanged {
        from: RunStatus,
        to: RunStatus,
        reason: Option<String>,
    },
    TurnStarted {
        turn: TurnNo,
        cause: TurnCause,
    },
    TurnEnded {
        turn: TurnNo,
    },
    OperationStarted {
        op_id: OperationId,
        kind: OperationKind,
        effect_id: Option<String>,
        idempotency_key: String,
        replay_safe: bool,
        interruptible: bool,
        for_call_id: Option<CallId>,
        attempt: u32,
    },
    OperationCompleted {
        op_id: OperationId,
        #[serde(default)]
        kind: Option<OperationKind>,
        #[serde(default)]
        effect_id: Option<String>,
        #[serde(default)]
        for_call_id: Option<CallId>,
        #[serde(default)]
        tool: Option<String>,
        outcome: OpOutcome,
        usage: Option<OpUsage>,
        result_ref: Option<ResultRef>,
        /// ModelTurn only: the call ids the model asked for.
        tool_calls: Vec<CallId>,
        /// A failed, retryable or blocked operation: what went wrong, in words
        /// safe to show a person (see [`OpFailure`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<OpFailure>,
    },
    OperationCancelled {
        op_id: OperationId,
        /// False: a non-interruptible op ran to its end, or the lease expired.
        acknowledged: bool,
        #[serde(default)]
        effect_id: Option<String>,
    },
    OperationAbandoned {
        op_id: OperationId,
        reason: String,
        #[serde(default)]
        effect_id: Option<String>,
        #[serde(default)]
        for_call_id: Option<CallId>,
    },
    ControlReceived {
        control_id: ControlId,
        command: ControlKind,
        issued_by: ActorRef,
    },
    ControlApplied {
        control_id: ControlId,
    },
    ControlRejected {
        control_id: ControlId,
        reason: String,
    },
    SteerQueued {
        steer_id: SteerId,
    },
    SteerConsumed {
        steer_id: SteerId,
        turn: TurnNo,
        /// The steer's input, so the log alone can deliver it.
        #[serde(default)]
        input: Value,
    },
    SteerDiscarded {
        steer_id: SteerId,
    },
    RequestOpened {
        request: PendingRequest,
    },
    RequestResolved {
        request_id: RequestId,
        resolution: Value,
        by_control: Option<ControlId>,
    },
    RequestClosed {
        request_id: RequestId,
        reason: String,
    },
    Resumed {
        from_checkpoint_no: Option<u32>,
    },
    CheckpointWritten {
        checkpoint_no: u32,
        at_seq: Seq,
        reason: CheckpointReason,
    },
    BudgetExceeded {
        which: String,
        limit: u64,
        used: u64,
    },
    LeaseAcquired {
        owner: String,
        epoch: LeaseEpoch,
    },
    LeaseTakenOver {
        from_owner: String,
        from_epoch: LeaseEpoch,
        owner: String,
        epoch: LeaseEpoch,
    },
    LeaseReleased {
        epoch: LeaseEpoch,
    },
    DomainApplied {
        state_rev: u64,
        delivered_seq: Seq,
        effects: Vec<Value>,
    },
    Terminal {
        status: TerminalStatus,
        outcome: RunOutcome,
    },
    /// Parent log: a child was admitted (its budgets reserved).
    ChildSpawned {
        child_run_id: RunId,
        child_no: u32,
        title: String,
        reserved: RunBudgets,
        depth: u8,
    },
    /// Parent log: a child's hand-back landed in the mailbox.
    ChildHandback {
        child_run_id: RunId,
        child_no: u32,
        status: RunStatus,
        outcome_kind: String,
        result_key: String,
        mail_no: u64,
        contract_satisfied: bool,
    },
    /// Child log: its parent has the hand-back.
    HandbackDelivered {
        parent_run_id: RunId,
    },
    /// Parent log: a child posted a message.
    MailPosted {
        mail_no: u64,
        from_run: RunId,
        result_key: String,
    },
    /// Mailbox items up to `up_to` were acknowledged.
    MailboxAcked {
        up_to: u64,
    },
    /// Parent log: the parent controlled a child (message, steer, interrupt).
    ChildControlled {
        child_run_id: RunId,
        action: String,
        control_id: ControlId,
        ack: String,
    },
    /// What waited for this run (a flow step) has its result.
    WaiterNotified {
        kind: String,
        target: String,
    },
}

impl RunEventKind {
    /// The snake_case type tag.
    pub fn type_name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod op_failure_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_provider_timeout_is_classed_and_its_internals_are_not_repeated() {
        let f = OpFailure::from_result(
            OpOutcome::Retryable,
            Some(&json!({
                "error_class": "provider",
                "message": "AI fallback failed: API request failed: HTTP 504 Gateway Timeout: class=timeout"
            })),
        )
        .unwrap();
        assert_eq!(f.class, "timeout");
        assert!(!f.message.contains("504") && !f.message.contains("fallback"));
    }

    #[test]
    fn a_declared_class_is_kept_and_success_has_none() {
        let f = OpFailure::from_result(OpOutcome::Failed, Some(&json!({"error_class": "config"})))
            .unwrap();
        assert_eq!(f.class, "config");
        assert!(OpFailure::from_result(OpOutcome::Succeeded, None).is_none());
        assert!(OpFailure::from_result(OpOutcome::Cancelled, None).is_none());
        let g = OpFailure::from_result(
            OpOutcome::Retryable,
            Some(&json!({"error_class": "provider", "message": "HTTP 429 Too Many Requests"})),
        )
        .unwrap();
        assert_eq!(g.class, "rate_limit");
    }
}
