// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The rules every reducer response and every tool result must satisfy.
//!
//! There is exactly one implementation, called by core, by the WebAssembly
//! adapter and by guest reducers themselves (a guest validates its own output
//! before returning it, so a guest bug surfaces as an honest refusal instead of
//! an illegal effect).
//!
//! | Code | Rule |
//! |---|---|
//! | R1 `contract_mismatch` | `resp.contract` is not in `req.accept` |
//! | R2 `state_rev_regressed` | `resp.state_rev ∉ {req.state_rev, req.state_rev + 1}` |
//! | R3 `effect_id_mismatch` | effect `i` must have id `"{resp.state_rev}:{i}"` |
//! | R4 `multiple_operations` | more than one operation effect |
//! | R5 `terminal_not_exclusive` | `complete`/`fail` with an operation effect, or twice |
//! | R6 `gap_without_follow_up` | `capability_gap` not followed by `complete{blocked}` or `ask_user` |
//! | R7 `effects_after_terminal` | a terminal run, or a `stopped` event: only `checkpoint` effects |
//! | R8 `unknown_effect_kind` | an effect kind outside the closed v1 set |
//! | R9 `state_too_large` | canonical state over [`crate::MAX_STATE_BYTES`] |
//! | R10 `effects_without_state_change` | effects require `state_rev == req.state_rev + 1` |
//! | R11 `open_requests_unresolved` | `user_input` with open requests: withdraw them all, or start no operation |
//! | R12 `unanswered_tool_calls` | a model turn must answer every unanswered call; `call_tool.for_call_id` must name one |
//! | R13 `projection_invalid` | `projection` present but not the generic shape |

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::canonical::canonical_json;
use crate::projection::PlanProjection;
use crate::reducer::{
    effect_id, CompleteOutcome, EffectBody, EventKind, ReducerRequest, ReducerResponse,
};
use crate::tool_result::{ArtifactRole, ToolResultEnvelope, ToolStatus};
use crate::{MAX_STATE_BYTES, TOOL_RESULT_V1};

/// A rule violation: a stable code and a human message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// Stable code (e.g. `multiple_operations`).
    pub code: String,
    /// Human message.
    pub message: String,
}

impl Refusal {
    /// Build a refusal.
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Refusal {}

fn refuse<T>(code: &str, message: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal::new(code, message))
}

/// Check a reducer response against the request it answers (R1–R13).
///
/// A response carrying `refused` is checked only for R1: the caller fails the
/// run with the reducer's own code.
pub fn validate_response(req: &ReducerRequest, resp: &ReducerResponse) -> Result<(), Refusal> {
    // R1
    if !req.accept.iter().any(|c| c == &resp.contract) {
        return refuse(
            "contract_mismatch",
            format!("response contract '{}' was not offered", resp.contract),
        );
    }
    if resp.refused.is_some() {
        return Ok(());
    }
    // R8 — before anything reads effect bodies.
    if let Some((i, _)) = resp
        .effects
        .iter()
        .enumerate()
        .find(|(_, e)| matches!(e.body, EffectBody::Unknown))
    {
        return refuse(
            "unknown_effect_kind",
            format!("effect {i} has an unknown kind"),
        );
    }
    // R2
    if resp.state_rev != req.state_rev && resp.state_rev != req.state_rev + 1 {
        return refuse(
            "state_rev_regressed",
            format!(
                "state_rev {} is neither {} nor {}",
                resp.state_rev,
                req.state_rev,
                req.state_rev + 1
            ),
        );
    }
    // R3
    for (i, effect) in resp.effects.iter().enumerate() {
        let want = effect_id(resp.state_rev, i);
        if effect.effect_id != want {
            return refuse(
                "effect_id_mismatch",
                format!(
                    "effect {i} has id '{}', expected '{want}'",
                    effect.effect_id
                ),
            );
        }
    }
    // R10
    if !resp.effects.is_empty() && resp.state_rev != req.state_rev + 1 {
        return refuse(
            "effects_without_state_change",
            "a response with effects must advance state_rev by one",
        );
    }
    // R4
    let operations = resp.effects.iter().filter(|e| e.is_operation()).count();
    if operations > 1 {
        return refuse(
            "multiple_operations",
            format!("{operations} operation effects; at most one per response"),
        );
    }
    // R5
    let terminals = resp.effects.iter().filter(|e| e.is_terminal()).count();
    if terminals > 1 || (terminals == 1 && operations > 0) {
        return refuse(
            "terminal_not_exclusive",
            "complete/fail must be the only terminal effect and cannot start an operation",
        );
    }
    // R6
    for (i, effect) in resp.effects.iter().enumerate() {
        if matches!(effect.body, EffectBody::CapabilityGap { .. }) {
            let followed = resp.effects[i + 1..].iter().any(|e| {
                matches!(
                    e.body,
                    EffectBody::AskUser { .. }
                        | EffectBody::Complete {
                            outcome: CompleteOutcome::Blocked,
                            ..
                        }
                )
            });
            if !followed {
                return refuse(
                    "gap_without_follow_up",
                    "capability_gap must be followed by complete{blocked} or ask_user",
                );
            }
        }
    }
    // R7
    if req.run.status.is_terminal() || req.event.kind == EventKind::Stopped {
        if let Some(e) = resp
            .effects
            .iter()
            .find(|e| !matches!(e.body, EffectBody::Checkpoint { .. }))
        {
            return refuse(
                "effects_after_terminal",
                format!(
                    "'{}' after the run ended; only checkpoint is allowed",
                    e.kind_name()
                ),
            );
        }
    }
    // R11
    if req.event.kind == EventKind::UserInput && !req.run.open_requests.is_empty() && operations > 0
    {
        let withdrawn: BTreeSet<&str> = resp
            .effects
            .iter()
            .filter_map(|e| match &e.body {
                EffectBody::WithdrawRequest { request_id } => Some(request_id.as_str()),
                _ => None,
            })
            .collect();
        if let Some(open) = req
            .run
            .open_requests
            .iter()
            .find(|r| !withdrawn.contains(r.request_id.as_str()))
        {
            return refuse(
                "open_requests_unresolved",
                format!(
                    "request '{}' is still open; withdraw it or start no operation",
                    open.request_id
                ),
            );
        }
    }
    // R12
    let unanswered: BTreeSet<&str> = req
        .run
        .unanswered_calls
        .iter()
        .map(String::as_str)
        .collect();
    let answered_by_call: BTreeSet<&str> = resp
        .effects
        .iter()
        .filter_map(|e| match &e.body {
            EffectBody::CallTool {
                for_call_id: Some(id),
                ..
            } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    for id in &answered_by_call {
        if !unanswered.contains(id) {
            return refuse(
                "unanswered_tool_calls",
                format!("call_tool answers '{id}', which is not an unanswered call"),
            );
        }
    }
    for effect in &resp.effects {
        if let EffectBody::RequestModelTurn { tool_results, .. } = &effect.body {
            let given: BTreeSet<&str> = tool_results.iter().map(|r| r.call_id.as_str()).collect();
            if let Some(missing) = unanswered
                .iter()
                .find(|id| !given.contains(*id) && !answered_by_call.contains(*id))
            {
                return refuse(
                    "unanswered_tool_calls",
                    format!("model turn requested while call '{missing}' is unanswered"),
                );
            }
        }
    }
    // R9
    let size = canonical_json(&resp.state).len();
    if size > MAX_STATE_BYTES {
        return refuse(
            "state_too_large",
            format!("state is {size} bytes, limit {MAX_STATE_BYTES}"),
        );
    }
    // R13
    if let Some(projection) = &resp.projection {
        if let Err(e) = PlanProjection::parse(projection) {
            return refuse("projection_invalid", e.to_string());
        }
    }
    Ok(())
}

/// Check a tool result envelope.
///
/// - `waiting` needs a `resume_key`;
/// - a `succeeded`, non-legacy result with writes carries exactly one
///   `artifact_refs[role=primary]`;
/// - every evidence subject revision names its `alg`.
pub fn validate_tool_result(env: &ToolResultEnvelope) -> Result<(), Refusal> {
    if env.envelope != TOOL_RESULT_V1 {
        return refuse(
            "envelope_mismatch",
            format!("envelope '{}' is not {TOOL_RESULT_V1}", env.envelope),
        );
    }
    if env.status == ToolStatus::Waiting && env.resume_key.as_deref().is_none_or(str::is_empty) {
        return refuse(
            "waiting_without_resume_key",
            "status waiting requires resume_key",
        );
    }
    if env.status == ToolStatus::Succeeded && !env.legacy && !env.writes.is_empty() {
        let primaries = env
            .artifact_refs
            .iter()
            .filter(|a| a.role == ArtifactRole::Primary)
            .count();
        if primaries != 1 {
            return refuse(
                "writes_without_primary",
                format!(
                    "a result with writes needs exactly one primary artifact_ref, has {primaries}"
                ),
            );
        }
    }
    if let Some(e) = env
        .evidence
        .iter()
        .find(|e| e.subject.revision.alg.is_empty())
    {
        return refuse(
            "evidence_revision_without_alg",
            format!("evidence of kind '{}' has a revision with no alg", e.kind),
        );
    }
    Ok(())
}
