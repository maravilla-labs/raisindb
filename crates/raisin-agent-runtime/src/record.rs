// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The run aggregate and the invariants the type cannot express.
//!
//! The record is kept SMALL: domain state and projection live write-once under
//! their own keys (the record keeps only `state_rev`), and consumed steers and
//! closed requests leave the record — their history is in the event log. A
//! lease renewal therefore rewrites a few KB, not a 256 KiB state.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::DomainBinding;
use crate::ids::{
    CallId, ControlId, LeaseEpoch, OperationId, Principal, RequestId, RunId, RunScope, Seq,
    SteerId, SubjectRef, TurnNo, Version,
};
use crate::state::RunState;

/// Bound on queued steers; one more is `ControlRejected{steer_queue_full}`.
pub const STEER_QUEUE_LIMIT: usize = 32;

/// The run aggregate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRunRecord {
    /// Id.
    pub run_id: RunId,
    /// Scope.
    pub scope: RunScope,
    /// What the run is about.
    pub subject: SubjectRef,
    /// Who it executes as.
    pub principal: Principal,
    /// `blake3` hex of the control capability given at create.
    pub control_capability_hash: Option<String>,
    /// Opaque agent reference; core never reads it.
    pub agent_ref: Option<String>,
    /// Idempotency key of the create.
    pub create_key: Option<String>,
    /// Lifecycle.
    pub state: RunState,
    /// Machine-readable reason for the current status.
    pub status_reason: Option<String>,
    /// +1 per commit.
    pub version: Version,
    /// Seq of the last persisted event.
    pub last_seq: Seq,
    /// Created, epoch ms.
    pub created_at_ms: u64,
    /// Last commit, epoch ms.
    pub updated_at_ms: u64,
    /// Monotonic counters.
    pub counters: Counters,
    /// Open turn.
    pub current_turn: Option<TurnNo>,
    /// Fencing token.
    pub lease_epoch: LeaseEpoch,
    /// Queued steers only.
    pub steer_queue: Vec<SteerEntry>,
    /// Tool calls of the last model turn not yet answered.
    pub unanswered_calls: Vec<CallId>,
    /// Budgets.
    pub budgets: RunBudgets,
    /// Usage.
    pub usage: RunUsage,
    /// Seq of the last checkpoint.
    pub last_checkpoint_seq: Option<Seq>,
    /// Domain binding, when a reducer drives the run.
    pub domain: Option<DomainBinding>,
    /// The parent, for a child run.
    pub parent_run_id: Option<RunId>,
    /// The root of the run tree, for a child run.
    pub root_run_id: Option<RunId>,
    /// 0 for a root run, parent depth + 1 for a child.
    pub depth: u8,
    /// The typed objective, for a child run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<crate::child::Delegation>,
    /// Every child this run spawned (bounded by `MAX_CHILDREN_PER_RUN`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<crate::child::ChildLink>,
    /// Unacknowledged mailbox items (hand-backs and child messages).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mailbox: Vec<crate::child::MailboxItem>,
    /// Refs of the most recent LARGE operation results (a rolling window),
    /// so every checkpoint can reference them instead of their payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub large_results: Vec<crate::events::ResultRef>,
    /// Opaque configuration for the run's operation executors (e.g. which
    /// function performs a model turn). Set at create; core never reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_config: Option<serde_json::Value>,
    /// What waits for this run to end outside its tree (a flow step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiter: Option<crate::waiter::RunWaiter>,
}

/// Monotonic counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Counters {
    /// Turns started.
    pub turn: u32,
    /// Operations started.
    pub op: u64,
    /// Steers queued.
    pub steer: u64,
    /// Requests opened.
    pub request: u64,
    /// Checkpoints written.
    pub checkpoint: u32,
    /// Children spawned.
    #[serde(default)]
    pub child: u32,
    /// Mailbox items received.
    #[serde(default)]
    pub mail: u64,
}

/// What kind of work an operation is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// A model call. Costs tokens, no side effects: replay-safe.
    ModelTurn,
    /// A tool call.
    ToolCall,
    /// Context compaction: replay-safe.
    Compaction,
    /// Anything else; not replay-safe unless the requester says so.
    Custom(String),
}

/// An operation in flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveOperation {
    /// Id; also the idempotency key.
    pub op_id: OperationId,
    /// Kind.
    pub kind: OperationKind,
    /// The domain effect that asked for it.
    pub effect_id: Option<String>,
    /// Idempotency key (== `op_id`).
    pub idempotency_key: String,
    /// Whether a takeover may re-dispatch it.
    pub replay_safe: bool,
    /// Whether a stop may interrupt it.
    pub interruptible: bool,
    /// The model tool call it answers.
    pub for_call_id: Option<CallId>,
    /// What to execute (tool path + args, or the model-turn request). Kept so a
    /// takeover can re-dispatch without the requester.
    #[serde(default)]
    pub input: Option<Value>,
    /// Started, epoch ms.
    pub started_at_ms: u64,
    /// Deadline, epoch ms.
    pub deadline_ms: Option<u64>,
    /// Epoch under which it was (re)started.
    pub lease_epoch: LeaseEpoch,
    /// 1 first dispatch, +1 per takeover re-dispatch.
    pub attempt: u32,
}

impl ActiveOperation {
    /// The tool path, for a tool call whose input names one.
    pub fn tool(&self) -> Option<String> {
        self.input
            .as_ref()
            .and_then(|i| i.get("tool"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
}

/// An execution lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// Holder.
    pub owner: String,
    /// Equals the record's `lease_epoch` while held.
    pub epoch: LeaseEpoch,
    /// Expiry, epoch ms.
    pub expires_at_ms: u64,
}

/// An open request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingRequest {
    /// Id.
    pub request_id: RequestId,
    /// What is asked.
    pub kind: PendingKind,
    /// The operation that asked, if any.
    pub requested_by: Option<OperationId>,
    /// The domain effect that asked, if any.
    pub effect_id: Option<String>,
    /// Seq of `RequestOpened`.
    pub created_seq: Seq,
    /// Expiry, epoch ms.
    pub expires_at_ms: Option<u64>,
}

/// What a request asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingKind {
    /// An approval bound to one digest.
    Approval {
        /// Digest an Approve must name.
        subject_digest: String,
        /// Algorithm.
        digest_alg: String,
        /// Summary.
        summary: String,
        /// Scope label.
        scope: Option<String>,
        /// Changes.
        changes: Option<Value>,
    },
    /// Input from the user.
    Input {
        /// Question.
        prompt: String,
        /// Choices.
        choices: Option<Vec<Value>>,
        /// Schema.
        schema: Option<Value>,
    },
    /// Waiting for a child's hand-back (resolved when it lands).
    Child {
        /// The child.
        child_run_id: RunId,
    },
    /// A tool returned `waiting`; resolved by `deliver_external_result`.
    External {
        /// The operation that parked.
        op_id: OperationId,
        /// Key the delivery names.
        resume_key: String,
        /// The model tool call the parked operation answers, so the delivered
        /// `tool_result` names it like a direct completion would.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        for_call_id: Option<CallId>,
        /// The tool that parked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
    },
}

impl PendingKind {
    /// `approval`, `input`, `external` or `child`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Approval { .. } => "approval",
            Self::Input { .. } => "input",
            Self::External { .. } => "external",
            Self::Child { .. } => "child",
        }
    }
}

/// A queued steer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SteerEntry {
    /// Id.
    pub steer_id: SteerId,
    /// The control that queued it.
    pub control_id: ControlId,
    /// The input.
    pub input: Value,
    /// Seq of `SteerQueued`.
    pub queued_seq: Seq,
}

/// What happens when a budget is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPolicy {
    /// Pause with a checkpoint (default).
    #[default]
    Pause,
    /// Fail the run.
    Fail,
}

/// Limits. `None` means unlimited.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunBudgets {
    /// Turns.
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Operations.
    #[serde(default)]
    pub max_operations: Option<u64>,
    /// Model calls.
    #[serde(default)]
    pub max_model_calls: Option<u32>,
    /// Input + output tokens.
    #[serde(default)]
    pub max_total_tokens: Option<u64>,
    /// Wall time since create.
    #[serde(default)]
    pub max_wall_ms: Option<u64>,
    /// Consecutive failed operations.
    #[serde(default)]
    pub max_consecutive_op_failures: Option<u32>,
    /// Children this run may spawn in total.
    #[serde(default)]
    pub max_children: Option<u32>,
    /// Children that may be live at once.
    #[serde(default)]
    pub max_live_children: Option<u32>,
    /// Deepest descendant depth (absolute; capped by `child::MAX_DEPTH`).
    #[serde(default)]
    pub max_depth: Option<u8>,
    /// Policy.
    #[serde(default)]
    pub on_exceeded: BudgetPolicy,
}

/// Usage counters, updated in the same commit as `OperationCompleted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunUsage {
    /// Turns.
    pub turns: u32,
    /// Operations finished.
    pub operations: u64,
    /// Model calls.
    pub model_calls: u32,
    /// Tool calls.
    pub tool_calls: u32,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Consecutive failed operations.
    pub consecutive_op_failures: u32,
    /// Tokens consumed by finished children (their whole subtree).
    #[serde(default)]
    pub child_input_tokens: u64,
    /// See `child_input_tokens`.
    #[serde(default)]
    pub child_output_tokens: u64,
    /// Operations finished by children (their whole subtree).
    #[serde(default)]
    pub child_operations: u64,
    /// Model calls made by children (their whole subtree).
    #[serde(default)]
    pub child_model_calls: u32,
    /// Tool calls made by children (their whole subtree).
    #[serde(default)]
    pub child_tool_calls: u32,
    /// Children handed back.
    #[serde(default)]
    pub children_completed: u32,
}

impl RunUsage {
    /// This run's usage plus everything its finished children consumed: what
    /// a hand-back reports to the next parent up.
    pub fn tree_total(&self) -> RunUsage {
        RunUsage {
            turns: self.turns,
            operations: self.operations + self.child_operations,
            model_calls: self.model_calls + self.child_model_calls,
            tool_calls: self.tool_calls + self.child_tool_calls,
            input_tokens: self.input_tokens + self.child_input_tokens,
            output_tokens: self.output_tokens + self.child_output_tokens,
            consecutive_op_failures: self.consecutive_op_failures,
            children_completed: self.children_completed,
            ..RunUsage::default()
        }
    }
}

/// An invariant the record violates.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invariant {code} violated: {message}")]
pub struct InvariantViolation {
    /// `I1`…`I6`, or `reserved`.
    pub code: &'static str,
    /// Detail.
    pub message: String,
}

fn violation(code: &'static str, message: impl Into<String>) -> Result<(), InvariantViolation> {
    Err(InvariantViolation {
        code,
        message: message.into(),
    })
}

/// The invariants the type cannot express, checked inside every commit.
///
/// `prev` is the stored record the commit replaces (none on create).
pub fn check_invariants(
    prev: Option<&AgentRunRecord>,
    next: &AgentRunRecord,
) -> Result<(), InvariantViolation> {
    // I1
    if let Some(lease) = next.state.lease() {
        if lease.epoch != next.lease_epoch {
            return violation("I1", "lease epoch differs from record lease_epoch");
        }
    }
    if let Some(op) = next.state.active_op() {
        if op.lease_epoch > next.lease_epoch {
            return violation("I1", "active op started under a future epoch");
        }
    }
    // I3
    if let Some(prev) = prev {
        let (p, n) = (&prev.counters, &next.counters);
        if n.turn < p.turn
            || n.op < p.op
            || n.steer < p.steer
            || n.request < p.request
            || n.checkpoint < p.checkpoint
        {
            return violation("I3", "a counter decreased");
        }
        if next.lease_epoch < prev.lease_epoch || next.version < prev.version {
            return violation("I3", "lease_epoch or version decreased");
        }
        if let (Some(pd), Some(nd)) = (&prev.domain, &next.domain) {
            if nd.state_rev < pd.state_rev || nd.delivered_seq < pd.delivered_seq {
                return violation("I3", "domain state_rev or delivered_seq decreased");
            }
        }
        if prev.domain.is_some() && next.domain.is_none() {
            return violation("I3", "domain binding removed");
        }
    }
    // I4
    if next.state.is_terminal() {
        if !next.steer_queue.is_empty() || !next.unanswered_calls.is_empty() {
            return violation("I4", "terminal run keeps steers or unanswered calls");
        }
        if next.domain.as_ref().is_some_and(|d| d.outbox.is_some()) {
            return violation("I4", "terminal run keeps a domain outbox");
        }
    }
    // I5
    let open = next.state.open_requests();
    let mut ids: Vec<&RequestId> = open.iter().map(|r| &r.request_id).collect();
    ids.sort();
    ids.dedup();
    if ids.len() != open.len() {
        return violation("I5", "duplicate open request ids");
    }
    // I6
    if let Some(d) = &next.domain {
        if d.delivered_seq > next.last_seq {
            return violation("I6", "delivered_seq beyond last_seq");
        }
    }
    // Lineage: set together, never changed, bounded depth.
    let is_child = next.parent_run_id.is_some();
    if is_child != next.root_run_id.is_some() || is_child != (next.depth > 0) {
        return violation("I7", "parent, root and depth must be set together");
    }
    if next.depth > crate::child::MAX_DEPTH {
        return violation("I7", "run tree too deep");
    }
    if let Some(prev) = prev {
        if prev.parent_run_id != next.parent_run_id
            || prev.root_run_id != next.root_run_id
            || prev.depth != next.depth
        {
            return violation("I7", "lineage changed");
        }
        if next.children.len() < prev.children.len() {
            return violation("I7", "a child link was removed");
        }
    }
    if next.children.len() > crate::child::MAX_CHILDREN_PER_RUN {
        return violation("I8", "too many children");
    }
    if next.mailbox.len() > crate::child::MAILBOX_LIMIT {
        return violation("I8", "mailbox overflow");
    }
    Ok(())
}
