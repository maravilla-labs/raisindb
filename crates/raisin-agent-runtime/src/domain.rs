// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The domain-reducer seam: binding, trait, and the mapping from persisted
//! run events to reducer events.
//!
//! The durable log IS the reducer's inbox. The driver feeds the events after
//! `domain.delivered_seq`, one reducer call per deliverable event, and moves
//! `delivered_seq` only in the commit that stores the reducer's answer. A crash
//! at any point therefore neither loses nor duplicates a delivery.

use async_trait::async_trait;
use raisin_agent_contract::{
    Effect, EventKind, OpenRequestView, ReducerEvent, ReducerRequest, ReducerResponse, Refusal,
    RunView, CONTRACT_V1,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::{OpOutcome, RunEvent, RunEventKind};
use crate::ids::Seq;
use crate::record::{AgentRunRecord, OperationKind, PendingKind};
use crate::state::TerminalStatus;

/// `blake3` hex of a reducer's artifact — the value a run pins at bind time.
///
/// Language-neutral: callers hash whatever bytes define the function (a
/// component, or source text plus its module set) in a stable order.
pub fn artifact_hash(parts: &[&[u8]]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(&(p.len() as u64).to_be_bytes());
        h.update(p);
    }
    h.finalize().to_hex().to_string()
}

/// Which reducer drives a run, pinned by artifact hash at bind time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReducerRef {
    /// Function path.
    pub function_path: String,
    /// Handler name inside the component.
    pub handler: String,
    /// `blake3` hex of the component bytes at bind time.
    pub artifact_hash: String,
}

/// An operation effect accepted but not yet started (budget pause).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingEffect {
    /// The effect, as the reducer returned it.
    pub effect: Effect,
    /// The state revision that produced it.
    pub state_rev: u64,
}

/// A run's binding to its domain reducer. Small: the state itself lives
/// write-once under `dom\0{state_rev}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainBinding {
    /// The reducer.
    pub reducer: ReducerRef,
    /// Contract spoken (`raisin.agent-run.reducer/1`).
    pub contract: String,
    /// Current state revision (0 = no state yet).
    pub state_rev: u64,
    /// Highest run-event seq already delivered.
    pub delivered_seq: Seq,
    /// At most one undispatched operation effect.
    pub outbox: Option<PendingEffect>,
    /// Which `dom\0{rev}` holds the latest projection.
    pub projection_rev: Option<u64>,
    /// Set by every terminal commit until finalize stores a final projection.
    pub projection_superseded: bool,
    /// Finalize has delivered everything after the terminal commit.
    pub finalized: bool,
    /// A changed artifact hash awaiting `Resume{accept_reducer_change}`.
    #[serde(default)]
    pub pending_artifact_hash: Option<String>,
}

impl DomainBinding {
    /// A fresh binding to `reducer`.
    pub fn new(reducer: ReducerRef) -> Self {
        Self {
            reducer,
            contract: CONTRACT_V1.into(),
            state_rev: 0,
            delivered_seq: Seq(0),
            outbox: None,
            projection_rev: None,
            projection_superseded: false,
            finalized: false,
            pending_artifact_hash: None,
        }
    }
}

/// Why a reducer call produced no usable response.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReducerCallError {
    /// Transport failure (trap, timeout, missing component): pause, resumable.
    #[error("reducer unavailable: {0}")]
    Unavailable(String),
    /// The reducer refused (`refused` set, or the handler returned `Err`).
    #[error("reducer refused: {code}")]
    Refused {
        /// Code.
        code: String,
        /// Message.
        message: String,
    },
    /// The response broke the contract.
    #[error("reducer response invalid: {0}")]
    Invalid(Refusal),
    /// The resolved component bytes differ from the pinned hash.
    #[error("reducer changed")]
    Changed {
        /// Hash of the bytes now resolved.
        actual_hash: String,
    },
}

/// A domain reducer: `(state, event) -> (state', effects)`, pure.
#[async_trait]
pub trait DomainReducer: Send + Sync {
    /// Which reducer this is.
    fn reducer_ref(&self) -> &ReducerRef;
    /// One call.
    async fn reduce(&self, req: &ReducerRequest) -> Result<ReducerResponse, ReducerCallError>;
}

/// Loads a stored result payload for an event (implemented over the store).
#[async_trait]
pub trait ResultLoader: Send + Sync {
    /// JSON payload stored under `key`, if any.
    async fn load_json(&self, key: &str) -> Option<Value>;
}

/// Turn a persisted run event into the reducer event it delivers, if any.
pub async fn map_event(ev: &RunEvent, loader: &dyn ResultLoader) -> Option<ReducerEvent> {
    let mk = |kind, effect_id: Option<String>, op: Option<String>, data: Value| ReducerEvent {
        seq: ev.seq.0,
        kind,
        effect_id,
        operation_id: op,
        data,
    };
    match &ev.kind {
        RunEventKind::RunCreated { input, .. } => Some(mk(
            EventKind::RunStarted,
            None,
            None,
            json!({ "input": input }),
        )),
        RunEventKind::SteerConsumed {
            steer_id, input, ..
        } => Some(mk(
            EventKind::UserInput,
            None,
            None,
            json!({ "steer_id": steer_id, "input": input }),
        )),
        RunEventKind::OperationCompleted {
            op_id,
            kind,
            effect_id,
            for_call_id,
            tool,
            outcome,
            usage,
            result_ref,
            tool_calls,
        } => {
            let payload = match result_ref {
                Some(r) => loader.load_json(&r.key).await,
                None => None,
            };
            let op = Some(op_id.0.clone());
            match outcome {
                OpOutcome::Succeeded => match kind {
                    Some(OperationKind::ModelTurn) => {
                        let mut data = payload.unwrap_or_else(|| json!({}));
                        if let Some(obj) = data.as_object_mut() {
                            obj.entry("tool_calls").or_insert_with(|| json!(tool_calls));
                            obj.insert("usage".into(), json!(usage));
                        }
                        Some(mk(
                            EventKind::ModelTurnCompleted,
                            effect_id.clone(),
                            op,
                            data,
                        ))
                    }
                    _ => Some(mk(
                        EventKind::ToolResult,
                        effect_id.clone(),
                        op,
                        json!({ "call_id": for_call_id, "tool": tool, "envelope": payload }),
                    )),
                },
                OpOutcome::Failed | OpOutcome::Retryable | OpOutcome::Blocked => {
                    let class = payload
                        .as_ref()
                        .and_then(|p| p.get("error_class"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if matches!(kind, Some(OperationKind::ModelTurn)) {
                                "provider".into()
                            } else {
                                "tool_error".into()
                            }
                        });
                    let message = payload
                        .as_ref()
                        .and_then(|p| p.get("message"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    Some(mk(
                        EventKind::OperationFailed,
                        effect_id.clone(),
                        op,
                        json!({
                            "error_class": class, "retryable": *outcome == OpOutcome::Retryable,
                            "outcome_unknown": false, "message": message, "call_id": for_call_id,
                            "tool": tool, "envelope": payload,
                        }),
                    ))
                }
                OpOutcome::Waiting | OpOutcome::Cancelled => None,
            }
        }
        RunEventKind::RequestResolved {
            request_id,
            resolution,
            ..
        } => {
            let effect_id = resolution
                .get("effect_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if resolution.get("kind").and_then(Value::as_str) == Some("external") {
                let envelope = match resolution.get("result_key").and_then(Value::as_str) {
                    Some(k) => loader.load_json(k).await,
                    None => None,
                };
                let op = resolution
                    .get("op_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                // A child's hand-back answers the WAITING operation: it names
                // that operation, not the child it came from.
                let envelope = match (envelope, &op) {
                    (Some(mut env), Some(op))
                        if env
                            .get("operation_id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id.starts_with("child:")) =>
                    {
                        env["operation_id"] = json!(op);
                        Some(env)
                    }
                    (env, _) => env,
                };
                return Some(mk(
                    EventKind::ToolResult,
                    effect_id,
                    op,
                    json!({
                        "call_id": resolution.get("for_call_id"), "tool": resolution.get("tool"),
                        "envelope": envelope, "request_id": request_id,
                    }),
                ));
            }
            let mut data = resolution.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("request_id".into(), json!(request_id));
                obj.remove("effect_id");
            }
            Some(mk(EventKind::RequestResolved, effect_id, None, data))
        }
        RunEventKind::RequestClosed { request_id, reason } if reason != "withdrawn" => Some(mk(
            EventKind::RequestClosed,
            None,
            None,
            json!({ "request_id": request_id, "reason": reason }),
        )),
        RunEventKind::OperationCancelled {
            op_id,
            acknowledged,
            effect_id,
        } => Some(mk(
            EventKind::OperationCancelled,
            effect_id.clone(),
            Some(op_id.0.clone()),
            json!({ "acknowledged": acknowledged }),
        )),
        RunEventKind::OperationAbandoned {
            op_id,
            reason,
            effect_id,
            for_call_id,
        } => Some(mk(
            EventKind::OperationFailed,
            effect_id.clone(),
            Some(op_id.0.clone()),
            json!({
                "error_class": "abandoned", "retryable": false, "outcome_unknown": true,
                "message": reason, "call_id": for_call_id,
            }),
        )),
        RunEventKind::Resumed { from_checkpoint_no } => Some(mk(
            EventKind::Resumed,
            None,
            None,
            json!({ "from_checkpoint_no": from_checkpoint_no }),
        )),
        RunEventKind::BudgetExceeded { which, limit, used } => Some(mk(
            EventKind::BudgetExceeded,
            None,
            None,
            json!({ "which": which, "limit": limit, "used": used }),
        )),
        RunEventKind::Terminal {
            status: TerminalStatus::Stopped,
            outcome,
        } => Some(mk(
            EventKind::Stopped,
            None,
            None,
            json!({ "reason": outcome.message }),
        )),
        _ => None,
    }
}

/// Core's view of the run for the reducer.
pub fn run_view(rec: &AgentRunRecord) -> RunView {
    RunView {
        run_id: rec.run_id.0.clone(),
        status: rec.state.status(),
        turn: rec.current_turn.map(|t| t.0),
        last_seq: rec.last_seq.0,
        usage: serde_json::to_value(rec.usage).unwrap_or(Value::Null),
        budgets: serde_json::to_value(&rec.budgets).unwrap_or(Value::Null),
        open_requests: rec
            .state
            .open_requests()
            .into_iter()
            .map(|r| OpenRequestView {
                request_id: r.request_id.0.clone(),
                kind: r.kind.name().into(),
                effect_id: r.effect_id.clone(),
                subject_digest: match &r.kind {
                    PendingKind::Approval { subject_digest, .. } => Some(subject_digest.clone()),
                    _ => None,
                },
                expires_at_ms: r.expires_at_ms,
            })
            .collect(),
        unanswered_calls: rec.unanswered_calls.iter().map(|c| c.0.clone()).collect(),
        scope: Some(json!({ "repo": rec.scope.repo_id, "branch": rec.scope.branch })),
        subject: serde_json::to_value(&rec.subject).ok(),
    }
}

/// Build the request delivering `event` against the stored `state`.
pub fn build_request(
    rec: &AgentRunRecord,
    binding: &DomainBinding,
    state: Option<Value>,
    event: ReducerEvent,
) -> ReducerRequest {
    ReducerRequest {
        contract: binding.contract.clone(),
        accept: vec![binding.contract.clone()],
        run: run_view(rec),
        state,
        state_rev: binding.state_rev,
        event,
    }
}
