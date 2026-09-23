// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reducer request and response envelopes (`raisin.agent-run.reducer/1`).
//!
//! Request: `(persisted domain state, one authoritative event)`. Every event is
//! derived from a PERSISTED run event and carries that event's sequence number;
//! nothing is synthesized in memory. Response: `(next state, effects[])`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::tool_result::Diagnostic;

/// Flat lifecycle status of a run, as indexes, events and the wire spell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Waiting for a driver to take the lease.
    Queued,
    /// A driver holds the lease.
    Running,
    /// Blocked on at least one open request (approval, input, external result).
    Waiting,
    /// Held by a control or a policy; resumable.
    Paused,
    /// Stop requested while an operation is in flight.
    Cancelling,
    /// Terminal: finished normally.
    Completed,
    /// Terminal: failed.
    Failed,
    /// Terminal: stopped by a control or by recovery.
    Stopped,
}

impl RunStatus {
    /// Whether the status is terminal (`completed`, `failed`, `stopped`).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Stopped)
    }

    /// The snake_case wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// One reducer invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerRequest {
    /// The contract this request is written in ([`crate::CONTRACT_V1`]).
    pub contract: String,
    /// Contract versions core accepts in the response.
    pub accept: Vec<String>,
    /// Core's view of the run.
    pub run: RunView,
    /// Opaque domain state; `null` on `run_started`.
    #[serde(default)]
    pub state: Option<Value>,
    /// Revision of `state`; `0` before the first response.
    pub state_rev: u64,
    /// The authoritative event being delivered.
    pub event: ReducerEvent,
}

/// Core's view of a run, as the reducer sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunView {
    /// Run id.
    pub run_id: String,
    /// Current lifecycle status.
    pub status: RunStatus,
    /// Current turn number, if a turn is open.
    #[serde(default)]
    pub turn: Option<u32>,
    /// Sequence number of the last persisted run event.
    pub last_seq: u64,
    /// Usage counters (core-defined shape, informational to the reducer).
    #[serde(default)]
    pub usage: Value,
    /// Budgets (core-defined shape, informational to the reducer).
    #[serde(default)]
    pub budgets: Value,
    /// Requests currently open on the run.
    #[serde(default)]
    pub open_requests: Vec<OpenRequestView>,
    /// Tool-call ids of the last model turn that are not answered yet.
    #[serde(default)]
    pub unanswered_calls: Vec<String>,
    /// Execution scope (`repo`, `branch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
    /// Opaque locator of what the run is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Value>,
}

/// An open request, as the reducer sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenRequestView {
    /// Request id.
    pub request_id: String,
    /// `approval`, `input` or `external`.
    pub kind: String,
    /// The effect that opened it, if a reducer did.
    #[serde(default)]
    pub effect_id: Option<String>,
    /// For an approval: the digest the decision must name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_digest: Option<String>,
    /// Expiry in epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

/// The closed v1 set of event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// From `RunCreated`.
    RunStarted,
    /// From `SteerConsumed`.
    UserInput,
    /// From `OperationCompleted` of a model turn.
    ModelTurnCompleted,
    /// From `OperationCompleted` of a tool call, or the resolution of an
    /// external request.
    ToolResult,
    /// From `RequestResolved` of an approval or input request.
    RequestResolved,
    /// From `RequestClosed`.
    RequestClosed,
    /// From a failed/retryable/blocked `OperationCompleted`, or `OperationAbandoned`.
    OperationFailed,
    /// From `OperationCancelled`.
    OperationCancelled,
    /// From `Resumed`.
    Resumed,
    /// From `BudgetExceeded`.
    BudgetExceeded,
    /// From `Terminal{Stopped}`.
    Stopped,
}

/// One authoritative event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerEvent {
    /// Sequence number of the run event that made this true.
    pub seq: u64,
    /// Event kind.
    pub kind: EventKind,
    /// The effect this event answers, when it answers one.
    #[serde(default)]
    pub effect_id: Option<String>,
    /// The operation this event is about, when there is one.
    #[serde(default)]
    pub operation_id: Option<String>,
    /// Kind-specific data (see the design's event table).
    #[serde(default = "empty_object")]
    pub data: Value,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

/// The reducer's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerResponse {
    /// The contract the reducer answered in.
    pub contract: String,
    /// Next domain state.
    pub state: Value,
    /// `request.state_rev` when unchanged, else `request.state_rev + 1`.
    pub state_rev: u64,
    /// Effects, in order. Effect `i` has id `"{state_rev}:{i}"`.
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// Optional plan projection (generic shape, see [`crate::projection`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    /// Diagnostics for humans and logs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// Set when the reducer refuses the request; the run fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<Refused>,
}

/// A reducer's refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    /// Stable code.
    pub code: String,
    /// Human message.
    #[serde(default)]
    pub message: String,
}

/// One effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// `"{state_rev}:{index}"`.
    pub effect_id: String,
    /// The typed body, tagged by `kind`.
    #[serde(flatten)]
    pub body: EffectBody,
}

impl Effect {
    /// Whether this effect starts an operation or opens a request (rule R4).
    pub fn is_operation(&self) -> bool {
        matches!(
            self.body,
            EffectBody::CallTool { .. }
                | EffectBody::RequestModelTurn { .. }
                | EffectBody::RequestApproval { .. }
                | EffectBody::AskUser { .. }
        )
    }

    /// Whether this effect ends the run.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.body,
            EffectBody::Complete { .. } | EffectBody::Fail { .. }
        )
    }

    /// The snake_case kind name.
    pub fn kind_name(&self) -> &'static str {
        self.body.kind_name()
    }
}

/// The closed v1 set of effect kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectBody {
    /// Execute a function tool.
    CallTool {
        /// Function path.
        tool: String,
        /// Arguments.
        args: Value,
        /// Whether the tool writes.
        mutating: bool,
        /// True only if the tool honours the operation id as an idempotency key.
        replay_safe: bool,
        /// Whether a stop may interrupt it (default true).
        #[serde(default = "yes")]
        interruptible: bool,
        /// Timeout in milliseconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        /// The model tool call this answers.
        #[serde(default)]
        for_call_id: Option<String>,
    },
    /// Ask the model for a turn.
    RequestModelTurn {
        /// Tools the model may call.
        #[serde(default)]
        tools_offered: Vec<ToolOffer>,
        /// Phase-scoped system addendum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        /// Context shaping hints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<Value>,
        /// Answers for every unanswered call id.
        #[serde(default)]
        tool_results: Vec<ToolResultEntry>,
        /// Structured-output schema.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_schema: Option<Value>,
    },
    /// Open an approval request bound to one changeset digest.
    RequestApproval {
        /// What is being approved.
        subject: ApprovalSubject,
        /// Expiry, relative.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_in_ms: Option<u64>,
    },
    /// Ask the user a question.
    AskUser {
        /// The question.
        question: String,
        /// Offered choices.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choices: Option<Vec<Value>>,
        /// Answer schema.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        /// Expiry, relative.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_in_ms: Option<u64>,
    },
    /// Withdraw an open request.
    WithdrawRequest {
        /// The request to withdraw.
        request_id: String,
    },
    /// Write a checkpoint.
    Checkpoint {
        /// Why.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Optional prose, never load-bearing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    /// Finish the run.
    Complete {
        /// `succeeded`, `partial` or `blocked`.
        outcome: CompleteOutcome,
        /// Summary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        /// Artifacts.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifacts: Vec<Value>,
        /// Evidence.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        evidence: Vec<Value>,
    },
    /// Fail the run.
    Fail {
        /// Stable code.
        code: String,
        /// Human message.
        message: String,
    },
    /// Report a structured capability gap. Must be followed by
    /// `complete{blocked}` or `ask_user` (R6).
    CapabilityGap {
        /// The gap.
        gap: CapabilityGap,
    },
    /// Any kind outside the closed v1 set (refused by R8).
    #[serde(other)]
    Unknown,
}

fn yes() -> bool {
    true
}

impl EffectBody {
    /// The snake_case kind name.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::CallTool { .. } => "call_tool",
            Self::RequestModelTurn { .. } => "request_model_turn",
            Self::RequestApproval { .. } => "request_approval",
            Self::AskUser { .. } => "ask_user",
            Self::WithdrawRequest { .. } => "withdraw_request",
            Self::Checkpoint { .. } => "checkpoint",
            Self::Complete { .. } => "complete",
            Self::Fail { .. } => "fail",
            Self::CapabilityGap { .. } => "capability_gap",
            Self::Unknown => "unknown",
        }
    }
}

/// Outcome of a `complete` effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompleteOutcome {
    /// Everything requested was done and proven.
    Succeeded,
    /// Some of it.
    Partial,
    /// Nothing more can be done without a human or a capability.
    Blocked,
}

/// A tool offered to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOffer {
    /// Tool name as the model sees it.
    pub name: String,
    /// `function` (core executes it) or `domain` (returned to the reducer).
    pub kind: ToolKind,
    /// Function path, for `function` tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_path: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Argument schema.
    pub schema: Value,
}

/// Who executes an offered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Core executes it.
    Function,
    /// Never executed by core; the call comes back to the reducer.
    Domain,
}

/// An answer to one model tool call, sent with the next model turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEntry {
    /// The call answered.
    pub call_id: String,
    /// True when the reducer answered it without executing anything.
    #[serde(default)]
    pub synthetic: bool,
    /// The answer.
    #[serde(default)]
    pub content: Value,
}

/// What an approval is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalSubject {
    /// Domain kind of the approved subject, e.g. `changeset`.
    pub kind: String,
    /// Digest the approval is bound to.
    pub digest: String,
    /// Digest algorithm (open string).
    pub digest_alg: String,
    /// Human summary.
    pub summary: String,
    /// Optional change list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<Value>>,
}

/// A structured capability gap (ADR "Capability gaps and escalation").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityGap {
    /// The outcome the user asked for.
    pub requested_outcome: String,
    /// What is missing.
    pub missing: String,
    /// What was inspected.
    #[serde(default)]
    pub inspected: Vec<Value>,
    /// Why what exists is not enough.
    pub why_insufficient: String,
    /// The proposed fix.
    pub proposed: ProposedFix,
    /// A safe alternative, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_alternative: Option<String>,
}

/// Where a capability gap should be fixed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedFix {
    /// Open string, e.g. `domain_package`, `agent_harness`, `core`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Build the deterministic effect id for effect `index` under `state_rev` (R3).
pub fn effect_id(state_rev: u64, index: usize) -> String {
    format!("{state_rev}:{index}")
}
