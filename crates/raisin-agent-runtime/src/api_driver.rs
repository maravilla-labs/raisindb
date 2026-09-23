// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The CLIENT-DRIVER half of the public run API: a run created without a
//! reducer is driven by its client, which holds the lease and records its own
//! operations. Re-exported from [`crate::api`], so callers use `api::begin`
//! etc. as before.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::{load_for, Caller, R};
use crate::events::{OpOutcome, OpUsage, TurnCause};
use crate::host::AgentRunHost;
use crate::ids::{CallId, LeaseEpoch, OperationId, RunId, RunScope};
use crate::lifecycle::{LeaseFence, NewRequest, OperationResult, OperationSpec};
use crate::record::{OperationKind, PendingKind};
use crate::state::{RunOutcome, RunStatus, TerminalStatus};

/// A lease held by a client driver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fence {
    /// Holder.
    pub owner: String,
    /// Fencing epoch.
    pub epoch: u64,
}

impl From<&Fence> for LeaseFence {
    fn from(f: &Fence) -> Self {
        LeaseFence {
            owner: f.owner.clone(),
            epoch: LeaseEpoch(f.epoch),
        }
    }
}

/// Take the lease of a queued client-driven run.
pub async fn acquire(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    owner: &str,
) -> R<Fence> {
    load_for(host, scope, run, caller).await?;
    let g = host.service().acquire_lease(scope, run, owner).await?;
    Ok(Fence {
        owner: g.fence.owner,
        epoch: g.fence.epoch.0,
    })
}

/// Renew it.
pub async fn renew(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    fence: &Fence,
) -> R<RunStatus> {
    load_for(host, scope, run, caller).await?;
    Ok(host
        .service()
        .renew_lease(scope, run, &fence.into())
        .await?
        .state
        .status())
}

/// Release it at a boundary.
pub async fn release(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    fence: &Fence,
) -> R<RunStatus> {
    load_for(host, scope, run, caller).await?;
    Ok(host
        .service()
        .release_lease(scope, run, &fence.into(), false)
        .await?
        .state
        .status())
}

/// Start an operation.
#[derive(Debug, Clone, Deserialize)]
pub struct BeginRequest {
    /// The lease.
    pub fence: Fence,
    /// `model_turn` | `tool_call` | `compaction` | anything else (custom).
    pub kind: String,
    /// What to execute (recorded; a takeover can re-dispatch it).
    #[serde(default)]
    pub input: Option<Value>,
    /// Whether a takeover may re-dispatch it.
    #[serde(default)]
    pub replay_safe: Option<bool>,
    /// Must finish once started.
    #[serde(default)]
    pub non_interruptible: bool,
    /// The model tool call it answers.
    #[serde(default)]
    pub for_call_id: Option<String>,
    /// For a model turn: the call ids its tool results answer.
    #[serde(default)]
    pub answers: Vec<String>,
}

/// Begin an operation under the lease.
pub async fn begin(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: BeginRequest,
) -> R<crate::record::ActiveOperation> {
    load_for(host, scope, run, caller).await?;
    let kind = match req.kind.as_str() {
        "model_turn" => OperationKind::ModelTurn,
        "tool_call" => OperationKind::ToolCall,
        "compaction" => OperationKind::Compaction,
        other => OperationKind::Custom(other.to_string()),
    };
    // Beginning an operation IS the idle boundary a client driver reaches, so
    // queued steers are consumed here exactly as the server driver consumes
    // them before its next step: the SteerConsumed events (with the input)
    // precede the operation that reads them, and the driver reads them from
    // the event log. Without this a client-driven run could never consume a
    // steer.
    let fence: crate::lifecycle::LeaseFence = (&req.fence).into();
    host.service().consume_steers(scope, run, &fence).await?;
    let spec = OperationSpec {
        kind: Some(kind),
        replay_safe: req.replay_safe,
        non_interruptible: req.non_interruptible,
        for_call_id: req.for_call_id.map(CallId),
        answers: req.answers.into_iter().map(CallId).collect(),
        input: req.input,
        cause: Some(TurnCause::Continuation),
        ..OperationSpec::default()
    };
    Ok(host
        .service()
        .begin_operation(scope, run, &(&req.fence).into(), spec)
        .await?
        .op)
}

/// Finish an operation.
#[derive(Debug, Clone, Deserialize)]
pub struct FinishRequest {
    /// The lease.
    pub fence: Fence,
    /// `succeeded` | `waiting` | `retryable` | `blocked` | `failed` | `cancelled`.
    pub outcome: OpOutcome,
    /// Result payload (stored beside the run).
    #[serde(default)]
    pub payload: Option<Value>,
    /// Model turn: the tool-call ids the model asked for.
    #[serde(default)]
    pub tool_calls: Vec<String>,
    /// Token usage.
    #[serde(default)]
    pub usage: Option<OpUsage>,
    /// For `waiting`: the key an external delivery will name.
    #[serde(default)]
    pub resume_key: Option<String>,
}

/// Record an operation's result.
pub async fn finish(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    op_id: &str,
    req: FinishRequest,
) -> R<RunStatus> {
    load_for(host, scope, run, caller).await?;
    let result = OperationResult {
        outcome: Some(req.outcome),
        usage: req.usage,
        payload: req.payload,
        tool_calls: req.tool_calls.into_iter().map(CallId).collect(),
        resume_key: req.resume_key,
    };
    let op = OperationId(op_id.to_string());
    Ok(host
        .service()
        .finish_operation(scope, run, &(&req.fence).into(), &op, result)
        .await?
        .state
        .status())
}

/// Open requests and wait.
#[derive(Debug, Clone, Deserialize)]
pub struct WaitRequest {
    /// The lease.
    pub fence: Fence,
    /// `[{kind:"approval"|"input", …, expires_at_ms?}]`.
    pub requests: Vec<WaitItem>,
}

/// One request to open.
#[derive(Debug, Clone, Deserialize)]
pub struct WaitItem {
    /// What it asks for.
    #[serde(flatten)]
    pub kind: PendingKind,
    /// Expiry.
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
}

/// Move the run to `Waiting`.
pub async fn wait(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: WaitRequest,
) -> R<RunStatus> {
    load_for(host, scope, run, caller).await?;
    let requests = req
        .requests
        .into_iter()
        .map(|w| NewRequest {
            kind: w.kind,
            effect_id: None,
            expires_at_ms: w.expires_at_ms,
        })
        .collect();
    Ok(host
        .service()
        .wait(scope, run, &(&req.fence).into(), requests)
        .await?
        .state
        .status())
}

/// End a client-driven run.
#[derive(Debug, Clone, Deserialize)]
pub struct CompleteRequest {
    /// The lease.
    pub fence: Fence,
    /// `completed` | `failed`.
    pub status: TerminalStatus,
    /// Outcome.
    pub outcome: RunOutcome,
}

/// End the run from its idle boundary.
pub async fn complete(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: CompleteRequest,
) -> R<RunStatus> {
    load_for(host, scope, run, caller).await?;
    Ok(host
        .service()
        .complete(
            scope,
            run,
            &(&req.fence).into(),
            req.status,
            req.outcome,
            None,
        )
        .await?
        .state
        .status())
}
