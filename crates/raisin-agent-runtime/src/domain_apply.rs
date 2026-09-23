// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Applying a VALIDATED reducer response (pure): state write, `DomainApplied`,
//! and the effect translation, in one transition.

use raisin_agent_contract::{CompleteOutcome, Effect, EffectBody, ReducerResponse};
use serde_json::{json, Value};

use crate::domain::PendingEffect;
use crate::events::{CheckpointReason, ResultRef, RunEventKind};
use crate::ids::{CallId, OperationId, RunId, Seq};
use crate::lifecycle::{
    begin_in, exceed, open_request, refusal, BeginRefusal, NewRequest, OperationSpec,
    TransitionRefusal,
};
use crate::record::{AgentRunRecord, OperationKind, PendingKind};
use crate::state::{NonEmpty, RunOutcome, RunState, TerminalStatus};
use crate::tx::{Transition, Tx};

/// The operation spec an effect asks for. `op_id` is the id the operation
/// will get, injected into tool args as `__raisin_context`.
fn spec_for(effect: &Effect, op_id: &OperationId, run: &RunId, now: u64) -> Option<OperationSpec> {
    match &effect.body {
        EffectBody::CallTool {
            tool,
            args,
            mutating,
            replay_safe,
            interruptible,
            timeout_ms,
            for_call_id,
        } => {
            // The reducer may hand a tool its own context (a conversation, a
            // workspace); core ADDS the run identity and always wins on it.
            let mut args = args.clone();
            if let Some(obj) = args.as_object_mut() {
                let mut ctx = match obj.remove("__raisin_context") {
                    Some(Value::Object(m)) => m,
                    _ => serde_json::Map::new(),
                };
                ctx.insert("run_id".into(), json!(run));
                ctx.insert("operation_id".into(), json!(op_id));
                obj.insert("__raisin_context".into(), Value::Object(ctx));
            }
            Some(OperationSpec {
                kind: Some(OperationKind::ToolCall),
                effect_id: Some(effect.effect_id.clone()),
                replay_safe: Some(*replay_safe),
                non_interruptible: !interruptible,
                for_call_id: for_call_id.clone().map(CallId),
                input: Some(json!({ "tool": tool, "args": args, "mutating": mutating })),
                deadline_ms: timeout_ms.map(|t| now + t),
                ..OperationSpec::default()
            })
        }
        EffectBody::RequestModelTurn { tool_results, .. } => Some(OperationSpec {
            kind: Some(OperationKind::ModelTurn),
            effect_id: Some(effect.effect_id.clone()),
            replay_safe: Some(true),
            answers: tool_results
                .iter()
                .map(|r| CallId(r.call_id.clone()))
                .collect(),
            input: serde_json::to_value(&effect.body).ok(),
            ..OperationSpec::default()
        }),
        _ => None,
    }
}

/// Start `effect` as an operation, or park it in the outbox if a budget
/// refuses (pausing or failing per policy).
pub(crate) fn start_or_park(
    tx: &mut Tx,
    effect: &Effect,
    state_rev: u64,
) -> Result<(), TransitionRefusal> {
    let op_id = OperationId::nth(&tx.rec.run_id, tx.rec.counters.op + 1);
    let run = tx.rec.run_id.clone();
    let spec = spec_for(effect, &op_id, &run, tx.now_ms).expect("operation effect");
    match begin_in(tx, spec) {
        Ok(_) => Ok(()),
        Err(BeginRefusal::Budget(e)) => {
            if let Some(d) = tx.rec.domain.as_mut() {
                d.outbox = Some(PendingEffect {
                    effect: effect.clone(),
                    state_rev,
                });
            }
            exceed(tx, &e);
            Ok(())
        }
        Err(BeginRefusal::ToolNotAllowed(tool)) => {
            // Not a contract error: the operation is recorded as BLOCKED by
            // policy, so the reducer (and the model behind it) can re-plan.
            let mut spec = spec_for(effect, &op_id, &run, tx.now_ms).expect("operation effect");
            spec.kind = Some(OperationKind::Custom("tool_denied".into()));
            let op = begin_in(tx, spec).map_err(|e| TransitionRefusal {
                code: "begin_refused".into(),
                message: format!("{e:?}"),
            })?;
            let result = crate::lifecycle::OperationResult {
                outcome: Some(crate::events::OpOutcome::Blocked),
                payload: Some(json!({
                    "error_class": "tool_not_allowed",
                    "message": format!("the child's objective does not allow tool '{tool}'"),
                })),
                ..Default::default()
            };
            crate::lifecycle_ops::completed(tx, &op, &result, crate::events::OpOutcome::Blocked);
            if let RunState::Running { lease, .. } = tx.rec.state.clone() {
                tx.rec.state = RunState::Running {
                    lease,
                    activity: crate::state::Activity::Idle { open: Vec::new() },
                };
            }
            Ok(())
        }
        Err(BeginRefusal::UnansweredCalls(m)) => refusal("unanswered_tool_calls", m),
        Err(BeginRefusal::OpenRequests) => refusal(
            "open_requests_unresolved",
            "an operation cannot start while requests are open",
        ),
        Err(other) => refusal("begin_refused", format!("{other:?}")),
    }
}

/// A domain-requested checkpoint preserves the DOMAIN's structured state — the
/// reducer's own state after this event — as the checkpoint's structured
/// state, stored beside the run like a compaction's. So resuming from a
/// checkpoint never depends on its prose summary: what the domain knew (its
/// spec, ledger, cursor, pending decisions) is in the checkpoint itself.
fn snapshot_domain_state(tx: &mut Tx, state: &Value) {
    let no = tx.rec.counters.checkpoint;
    let bytes = serde_json::to_vec(state).unwrap_or_default();
    let r = ResultRef {
        key: format!("ckpt:{no}:state"),
        bytes: bytes.len() as u64,
        content_type: "application/json".into(),
    };
    tx.results.push((r.clone(), bytes));
    if let Some(c) = tx.checkpoint.as_mut() {
        c.structured_ref = Some(r);
    }
}

/// Apply a VALIDATED response to event `ev_seq` (pure).
pub fn apply_domain_response(
    rec: &AgentRunRecord,
    ev_seq: Seq,
    resp: &ReducerResponse,
    finalize_last: Option<bool>,
    now: u64,
) -> Result<Transition, TransitionRefusal> {
    let mut tx = Tx::new(rec, now);
    let mut b = rec.domain.clone().ok_or_else(|| TransitionRefusal {
        code: "no_domain".into(),
        message: "run has no reducer".into(),
    })?;
    if resp.state_rev != b.state_rev {
        tx.domain_state = Some((
            resp.state_rev,
            json!({ "state": resp.state, "projection": resp.projection }),
        ));
        b.state_rev = resp.state_rev;
        if resp.projection.is_some() {
            b.projection_rev = Some(resp.state_rev);
            if finalize_last.is_some() {
                b.projection_superseded = false;
            }
        }
    }
    b.delivered_seq = ev_seq;
    if finalize_last == Some(true) {
        b.finalized = true;
    }
    let state_rev = b.state_rev;
    tx.rec.domain = Some(b);
    let effects: Vec<Value> = resp
        .effects
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect();
    tx.push(RunEventKind::DomainApplied {
        state_rev,
        delivered_seq: ev_seq,
        effects,
    });

    let mut operation: Option<&Effect> = None;
    let mut requests = Vec::new();
    let mut terminal = None;
    for effect in &resp.effects {
        match &effect.body {
            EffectBody::CallTool { .. } | EffectBody::RequestModelTurn { .. } => {
                operation = Some(effect)
            }
            EffectBody::RequestApproval {
                subject,
                expires_in_ms,
            } => requests.push(NewRequest {
                kind: PendingKind::Approval {
                    subject_digest: subject.digest.clone(),
                    digest_alg: subject.digest_alg.clone(),
                    summary: subject.summary.clone(),
                    scope: Some(subject.kind.clone()),
                    changes: subject.changes.clone().map(Value::Array),
                },
                effect_id: Some(effect.effect_id.clone()),
                expires_at_ms: expires_in_ms.map(|ms| now + ms),
            }),
            EffectBody::AskUser {
                question,
                choices,
                schema,
                expires_in_ms,
            } => requests.push(NewRequest {
                kind: PendingKind::Input {
                    prompt: question.clone(),
                    choices: choices.clone(),
                    schema: schema.clone(),
                },
                effect_id: Some(effect.effect_id.clone()),
                expires_at_ms: expires_in_ms.map(|ms| now + ms),
            }),
            EffectBody::WithdrawRequest { request_id } => {
                let id = crate::ids::RequestId(request_id.clone());
                if tx
                    .rec
                    .state
                    .open_requests()
                    .iter()
                    .any(|r| r.request_id == id)
                {
                    tx.push(RunEventKind::RequestClosed {
                        request_id: id.clone(),
                        reason: "withdrawn".into(),
                    });
                    tx.remove_request(&id);
                }
            }
            EffectBody::Checkpoint { summary, .. } => {
                tx.checkpoint(CheckpointReason::DomainRequested, summary.clone());
                snapshot_domain_state(&mut tx, &resp.state);
            }
            EffectBody::Complete {
                outcome,
                summary,
                artifacts,
                evidence,
            } => {
                let kind = match outcome {
                    CompleteOutcome::Succeeded => "succeeded",
                    CompleteOutcome::Partial => "partial",
                    CompleteOutcome::Blocked => "blocked",
                };
                let detail = json!({ "artifacts": artifacts, "evidence": evidence });
                terminal = Some((
                    TerminalStatus::Completed,
                    RunOutcome {
                        kind: kind.into(),
                        code: None,
                        message: summary.clone(),
                        detail: Some(detail),
                    },
                    None,
                ));
            }
            EffectBody::Fail { code, message } => {
                let outcome = RunOutcome {
                    kind: "failed".into(),
                    code: Some(code.clone()),
                    message: Some(message.clone()),
                    detail: None,
                };
                terminal = Some((TerminalStatus::Failed, outcome, Some(code.clone())));
            }
            EffectBody::CapabilityGap { .. } => {}
            EffectBody::Unknown => return refusal("unknown_effect_kind", "unknown effect"),
        }
    }
    if finalize_last.is_some() || tx.rec.state.is_terminal() {
        // R7 already restricted a terminal run to checkpoint effects.
        return Ok(tx.finish());
    }
    if let Some((status, outcome, reason)) = terminal {
        tx.terminate(status, outcome, reason);
    } else if let Some(effect) = operation {
        if !tx.rec.state.open_requests().is_empty() {
            return refusal(
                "open_requests_unresolved",
                "an operation cannot start while requests are open",
            );
        }
        start_or_park(&mut tx, effect, state_rev)?;
    } else {
        let mut open = tx.rec.state.open_requests();
        for req in requests {
            open.push(open_request(&mut tx, req));
        }
        if let Some(open) = NonEmpty::from_vec(open) {
            tx.set_state(RunState::Waiting { open }, None);
        }
    }
    Ok(tx.finish())
}
