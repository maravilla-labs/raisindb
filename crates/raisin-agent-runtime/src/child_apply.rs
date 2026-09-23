// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure transitions of the parent/child protocol after admission: the
//! hand-back (child → parent mailbox), child messages, mailbox acks, and the
//! parent's audit of what it did to a child.
//!
//! Every function here is idempotent on the record it transforms: a
//! redelivered hand-back finds its link already `delivered` and does nothing,
//! so the job queue and the sweeper may retry freely on any node.

use raisin_agent_contract::TOOL_RESULT_V1;
use serde_json::{json, Value};

use crate::child::{resume_key, MailKind, MailboxItem, MAILBOX_LIMIT, MAX_INLINE_BYTES};
use crate::events::{ResultRef, RunEventKind};
use crate::ids::{ControlId, RunId};
use crate::lifecycle::{refusal, TransitionRefusal};
use crate::record::{AgentRunRecord, PendingKind};
use crate::state::{RunOutcome, RunState, RunStatus, WakeReason};
use crate::tx::{Transition, Tx};

/// A child's terminal hand-back, as the parent stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Handback {
    /// The `raisin.tool-result/1` envelope (so a tool waiting on the child
    /// receives it as its result).
    pub envelope: Value,
    /// Child status.
    pub status: RunStatus,
    /// Outcome kind.
    pub outcome_kind: String,
    /// Whether the hand-back contract was met.
    pub satisfied: bool,
}

fn contract_violations(child: &AgentRunRecord, outcome: &RunOutcome) -> Vec<Value> {
    let Some(d) = &child.delegation else {
        return Vec::new();
    };
    let obj = &d.objective;
    let detail = outcome.detail.clone().unwrap_or(Value::Null);
    let mut out = Vec::new();
    for f in &obj.hand_back.required_fields {
        if detail.get(f).is_none_or(Value::is_null) {
            out.push(json!({ "code": "missing_field", "field": f }));
        }
    }
    let artifacts = detail
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, a) in obj
        .expected_artifacts
        .iter()
        .enumerate()
        .filter(|(_, a)| a.required)
    {
        let found = artifacts.iter().any(|got| {
            let kind_ok = got.get("kind").and_then(Value::as_str) == Some(a.kind.as_str());
            let loc_ok = a
                .locator
                .as_ref()
                .is_none_or(|l| got.get("locator") == Some(l) || got.get("path") == l.get("path"));
            kind_ok && loc_ok
        });
        if !found {
            out.push(json!({ "code": "missing_artifact", "index": i, "kind": a.kind }));
        }
    }
    let checks = detail
        .get("checks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for c in obj.acceptance_checks.iter().filter(|c| c.required) {
        let passed = checks.iter().any(|got| {
            got.get("id").and_then(Value::as_str) == Some(c.id.as_str())
                && got.get("passed").and_then(Value::as_bool) == Some(true)
        });
        if !passed {
            out.push(json!({ "code": "check_not_passed", "check": c.id }));
        }
    }
    out
}

/// Build the hand-back of a TERMINAL child.
pub fn build_handback(child: &AgentRunRecord) -> Option<Handback> {
    let RunState::Terminal { terminal, outcome } = &child.state else {
        return None;
    };
    let status = terminal.status();
    let violations = if status == RunStatus::Completed {
        contract_violations(child, outcome)
    } else {
        Vec::new()
    };
    let satisfied =
        status == RunStatus::Completed && violations.is_empty() && outcome.kind != "blocked";
    let tool_status = if satisfied {
        "succeeded"
    } else if status == RunStatus::Completed {
        "blocked"
    } else {
        "failed"
    };
    let mut diagnostics: Vec<Value> = violations
        .iter()
        .map(|v| json!({ "code": format!("handback_{}", v["code"].as_str().unwrap_or("violation")), "severity": "error", "message": v.to_string() }))
        .collect();
    if status != RunStatus::Completed {
        diagnostics.push(json!({
            "code": outcome.code.clone().unwrap_or_else(|| format!("child_{}", status.as_str())),
            "severity": "error",
            "message": outcome.message.clone().unwrap_or_else(|| format!("child run {}", status.as_str())),
        }));
    }
    let d = child.delegation.as_ref();
    let envelope = json!({
        "envelope": TOOL_RESULT_V1,
        "operation_id": resume_key(&child.run_id),
        "status": tool_status,
        "diagnostics": diagnostics,
        "payload": {
            "child_run_id": child.run_id, "child_no": d.map(|d| d.child_no),
            "title": d.map(|d| d.objective.title.clone()), "status": status,
            "outcome": outcome, "usage": child.usage.tree_total(),
            "contract": { "satisfied": satisfied, "violations": violations },
        },
    });
    Some(Handback {
        envelope,
        status,
        outcome_kind: outcome.kind.clone(),
        satisfied,
    })
}

/// Result key of a child's hand-back in the parent's results.
pub fn handback_key(child: &RunId) -> String {
    format!("handback:{child}")
}

/// Resolve every open request of `tx` waiting for the child whose hand-back
/// is stored under `result_key`: a waiting tool (`External` with resume key
/// `child:{id}`) gets it as its tool result; a `Child` wait gets a resolution.
pub(crate) fn resolve_child_waits(tx: &mut Tx, child: &RunId, result_key: &str, status: RunStatus) {
    let key = resume_key(child);
    for request in tx.rec.state.open_requests() {
        let resolution = match &request.kind {
            PendingKind::External {
                op_id,
                resume_key: k,
                for_call_id,
                tool,
            } if *k == key => json!({
                "kind": "external", "result_key": result_key, "op_id": op_id,
                "effect_id": request.effect_id, "delivery_id": key, "for_call_id": for_call_id, "tool": tool,
            }),
            PendingKind::Child { child_run_id } if child_run_id == child => json!({
                "kind": "child", "child_run_id": child, "status": status, "result_key": result_key,
                "effect_id": request.effect_id,
            }),
            _ => continue,
        };
        tx.push(RunEventKind::RequestResolved {
            request_id: request.request_id.clone(),
            resolution,
            by_control: None,
        });
        tx.remove_request(&request.request_id);
    }
    if let RunState::Queued { wake, .. } = &mut tx.rec.state {
        if tx.wake.is_some() {
            *wake = WakeReason::ExternalResult;
            tx.wake = Some(WakeReason::ExternalResult);
        }
    }
}

fn push_mail(
    tx: &mut Tx,
    kind: MailKind,
    from: &RunId,
    key: &str,
    status: Option<RunStatus>,
) -> (u64, u64) {
    tx.rec.counters.mail += 1;
    let mail_no = tx.rec.counters.mail;
    let seq = tx.next_seq().0;
    tx.rec.mailbox.push(MailboxItem {
        mail_no,
        kind,
        from_run: from.clone(),
        seq,
        result_key: key.into(),
        status,
    });
    (mail_no, seq)
}

fn store_json(tx: &mut Tx, key: &str, v: &Value) {
    let bytes = serde_json::to_vec(v).unwrap_or_default();
    tx.results.push((
        ResultRef {
            key: key.into(),
            bytes: bytes.len() as u64,
            content_type: "application/json".into(),
        },
        bytes,
    ));
}

/// Land a child's hand-back in the parent. `None` when it already landed
/// (or the parent has no link to this child).
pub fn apply_handback(
    parent: &AgentRunRecord,
    child: &AgentRunRecord,
    hb: &Handback,
    now: u64,
) -> Option<Transition> {
    let idx = parent
        .children
        .iter()
        .position(|l| l.run_id == child.run_id)?;
    if parent.children[idx].delivered {
        return None;
    }
    let mut tx = Tx::new(parent, now);
    let key = handback_key(&child.run_id);
    store_json(&mut tx, &key, &hb.envelope);
    let usage = child.usage.tree_total();
    {
        let u = &mut tx.rec.usage;
        u.child_input_tokens += usage.input_tokens;
        u.child_output_tokens += usage.output_tokens;
        u.child_operations += usage.operations;
        u.child_model_calls += usage.model_calls;
        u.child_tool_calls += usage.tool_calls;
        u.children_completed += 1;
    }
    // The mailbox is bounded by the child count for completions; if messages
    // filled it, the oldest acknowledged-by-nobody MESSAGE makes room.
    if tx.rec.mailbox.len() >= MAILBOX_LIMIT {
        if let Some(pos) = tx
            .rec
            .mailbox
            .iter()
            .position(|m| m.kind == MailKind::Message)
        {
            tx.rec.mailbox.remove(pos);
        }
    }
    let (mail_no, _) = push_mail(
        &mut tx,
        MailKind::Completion,
        &child.run_id,
        &key,
        Some(hb.status),
    );
    let link = &mut tx.rec.children[idx];
    link.delivered = true;
    link.status = hb.status;
    link.result_key = Some(key.clone());
    link.usage = Some(usage);
    let child_no = link.child_no;
    tx.push(RunEventKind::ChildHandback {
        child_run_id: child.run_id.clone(),
        child_no,
        status: hb.status,
        outcome_kind: hb.outcome_kind.clone(),
        result_key: key.clone(),
        mail_no,
        contract_satisfied: hb.satisfied,
    });
    if !tx.rec.state.is_terminal() {
        resolve_child_waits(&mut tx, &child.run_id, &key, hb.status);
    }
    Some(tx.finish())
}

/// Mark a child's hand-back delivered. `None` when already marked.
pub fn apply_handback_delivered(child: &AgentRunRecord, now: u64) -> Option<Transition> {
    let d = child.delegation.as_ref()?;
    if d.handback_delivered {
        return None;
    }
    let mut tx = Tx::new(child, now);
    if let Some(d) = tx.rec.delegation.as_mut() {
        d.handback_delivered = true;
    }
    tx.push(RunEventKind::HandbackDelivered {
        parent_run_id: child.parent_run_id.clone()?,
    });
    Some(tx.finish())
}

/// A child posts a message to its parent's mailbox.
pub fn apply_post_to_parent(
    parent: &AgentRunRecord,
    from: &RunId,
    message: &Value,
    message_id: &str,
    now: u64,
) -> Result<Transition, TransitionRefusal> {
    if !parent
        .children
        .iter()
        .any(|l| &l.run_id == from && l.is_live())
    {
        return refusal(
            "not_a_live_child",
            "only a live child may post to its parent",
        );
    }
    if serde_json::to_vec(message)
        .map(|b| b.len())
        .unwrap_or(usize::MAX)
        > MAX_INLINE_BYTES
    {
        return refusal(
            "message_too_large",
            format!("a message is bounded to {MAX_INLINE_BYTES} bytes"),
        );
    }
    if parent.mailbox.len() >= MAILBOX_LIMIT {
        return refusal(
            "mailbox_full",
            "the parent's mailbox is full; it must acknowledge first",
        );
    }
    let mut tx = Tx::new(parent, now);
    let key = format!("mail:{from}:{message_id}");
    store_json(
        &mut tx,
        &key,
        &json!({ "from_run": from, "message_id": message_id, "message": message }),
    );
    let (mail_no, _) = push_mail(&mut tx, MailKind::Message, from, &key, None);
    tx.push(RunEventKind::MailPosted {
        mail_no,
        from_run: from.clone(),
        result_key: key,
    });
    Ok(tx.finish())
}

/// Acknowledge mailbox items up to `up_to`. `None` when nothing to drop.
pub fn apply_ack(rec: &AgentRunRecord, up_to: u64, now: u64) -> Option<Transition> {
    if !rec.mailbox.iter().any(|m| m.mail_no <= up_to) {
        return None;
    }
    let mut tx = Tx::new(rec, now);
    tx.rec.mailbox.retain(|m| m.mail_no > up_to);
    tx.push(RunEventKind::MailboxAcked { up_to });
    Some(tx.finish())
}

/// Log, on the parent, a control it issued to a child.
pub fn apply_child_controlled(
    parent: &AgentRunRecord,
    child: &RunId,
    action: &str,
    control_id: &ControlId,
    ack: &str,
    now: u64,
) -> Transition {
    let mut tx = Tx::new(parent, now);
    if let Some(l) = tx.rec.children.iter_mut().find(|l| &l.run_id == child) {
        if l.is_live() && action == "interrupt" {
            l.status = RunStatus::Cancelling;
        }
    }
    tx.push(RunEventKind::ChildControlled {
        child_run_id: child.clone(),
        action: action.into(),
        control_id: control_id.clone(),
        ack: ack.into(),
    });
    tx.finish()
}

/// Refresh the parent's last-known status of a live child (no event).
pub fn apply_child_status(
    parent: &AgentRunRecord,
    child: &RunId,
    status: RunStatus,
    now: u64,
) -> Option<Transition> {
    let l = parent.children.iter().find(|l| &l.run_id == child)?;
    if !l.is_live() || l.status == status {
        return None;
    }
    let mut tx = Tx::new(parent, now);
    if let Some(l) = tx.rec.children.iter_mut().find(|l| &l.run_id == child) {
        l.status = status;
    }
    Some(tx.finish())
}
