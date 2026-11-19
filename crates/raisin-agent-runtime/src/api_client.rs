// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Client tools: the answer a browser (or any client) posts for a tool call
//! that ran on ITS side.
//!
//! A run's reducer offers the page's tools to the model as `page.<name>` and
//! dispatches a call to the built-in relay function, which tells the client
//! (a conversation event) and answers `waiting` under the resume key
//! `client:<call_id>` with an expiry. The client's answer lands here: the
//! caller must be allowed to access the run (its owner), and the open
//! `External` request must be one the relay opened, so this route can never
//! resolve an approval, an input request, a child wait or another tool's
//! external wait. The answer becomes a `raisin.tool-result/1` envelope and is
//! delivered through the ordinary external-result path (deduplicated by
//! `delivery_id`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{may_access, ApiError, Caller};
use crate::control::ControlAck;
use crate::host::AgentRunHost;
use crate::ids::{RunId, RunScope, SystemToken};
use crate::record::PendingKind;
use crate::service::AgentRunService;

/// The relay function a client-tool call is dispatched to.
pub const CLIENT_TOOL_RELAY: &str = "/lib/raisin/ai/client-tool-relay";
/// Every client call's resume key starts with this, followed by the call id.
pub const CLIENT_RESUME_PREFIX: &str = "client:";

/// The body of `POST /api/agent-runs/{repo}/{run}/client-result`.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientResultRequest {
    /// `client:<call_id>`.
    pub resume_key: String,
    /// Idempotency key of this delivery.
    pub delivery_id: String,
    /// `{ ok, content?, error?, untrusted? }`.
    pub result: Value,
    /// The page's CURRENT tools; replaces the run's set when present.
    #[serde(default)]
    pub client_tools: Option<Value>,
    /// Fresh page context; replaces the run's when present.
    #[serde(default)]
    pub client_context: Option<Value>,
}

/// `{ status: "applied" | "duplicate" | "rejected", reason? }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientResultAnswer {
    /// `applied`, `duplicate` or `rejected`.
    pub status: &'static str,
    /// Why a rejection: `request_not_open`, `not_a_client_call`,
    /// `forbidden` or `expired`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ClientResultAnswer {
    fn applied() -> Self {
        Self {
            status: "applied",
            reason: None,
        }
    }
    fn duplicate() -> Self {
        Self {
            status: "duplicate",
            reason: None,
        }
    }
    fn rejected(reason: &str) -> Self {
        Self {
            status: "rejected",
            reason: Some(reason.into()),
        }
    }
}

/// The tool-result envelope the reducer receives for a client answer.
fn envelope_of(req: &ClientResultRequest) -> Value {
    let ok = req.result.get("ok").and_then(Value::as_bool) == Some(true);
    let untrusted = req.result.get("untrusted").and_then(Value::as_bool) == Some(true);
    let mut payload = json!({ "ok": ok });
    if let Some(content) = req.result.get("content") {
        payload["content"] = content.clone();
    }
    if let Some(error) = req.result.get("error").filter(|e| !e.is_null()) {
        payload["error"] = error.clone();
    }
    let mut env = json!({
        "envelope": "raisin.tool-result/1",
        // Deterministic, so a repeated delivery has the same digest (a duplicate,
        // never a "delivery_id reused"); the runtime names the operation itself.
        "operation_id": req.resume_key,
        "status": if ok { "succeeded" } else { "failed" },
        "payload": payload,
        "untrusted": untrusted,
        "source": "client",
    });
    if let Some(tools) = req.client_tools.as_ref().filter(|t| t.is_array()) {
        env["client_tools"] = tools.clone();
    }
    if let Some(ctx) = req.client_context.as_ref().filter(|c| c.is_object()) {
        env["client_context"] = ctx.clone();
    }
    env
}

/// Deliver a client's answer. See the module docs for what is checked.
pub async fn client_result_on(
    svc: &AgentRunService,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: ClientResultRequest,
) -> Result<ClientResultAnswer, ApiError> {
    let rec = svc
        .get(scope, run)
        .await?
        .ok_or_else(|| ApiError::new(404, "not_found", "run not found"))?;
    if !may_access(&rec, caller) {
        return Ok(ClientResultAnswer::rejected("forbidden"));
    }
    if req.delivery_id.trim().is_empty() {
        return Err(ApiError::new(400, "invalid", "delivery_id is required"));
    }
    let call_id = req
        .resume_key
        .strip_prefix(CLIENT_RESUME_PREFIX)
        .filter(|id| !id.is_empty());
    if call_id.is_none() {
        return Ok(ClientResultAnswer::rejected("not_a_client_call"));
    }
    let open = rec.state.open_requests().into_iter().find(|r| {
        matches!(&r.kind, PendingKind::External { resume_key, .. } if *resume_key == req.resume_key)
    });
    // Not open: a repeat of an answer already applied is a duplicate (the
    // control id is remembered), anything else is `request_not_open`.
    if let Some(r) = &open {
        let PendingKind::External { tool, .. } = &r.kind else {
            unreachable!("matched above")
        };
        // Only a call the relay parked may be answered here.
        if tool.as_deref() != Some(CLIENT_TOOL_RELAY) {
            return Ok(ClientResultAnswer::rejected("not_a_client_call"));
        }
        // Past its deadline: the recovery sweep closes it, and the model is
        // told the client was unavailable; a late answer is refused.
        if r.expires_at_ms.is_some_and(|at| at <= svc.now()) {
            return Ok(ClientResultAnswer::rejected("expired"));
        }
    }
    let envelope = envelope_of(&req);
    let ack = svc
        .deliver_external_result(
            scope,
            run,
            &req.resume_key,
            &req.delivery_id,
            envelope,
            &SystemToken::in_process(),
        )
        .await?;
    Ok(match ack {
        ControlAck::Applied { .. } => ClientResultAnswer::applied(),
        ControlAck::Duplicate { .. } => ClientResultAnswer::duplicate(),
        ControlAck::Rejected { reason, .. } => ClientResultAnswer::rejected(&reason),
    })
}

/// [`client_result_on`] through the host.
pub async fn client_result(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: ClientResultRequest,
) -> Result<ClientResultAnswer, ApiError> {
    client_result_on(host.service(), scope, run, caller, req).await
}
