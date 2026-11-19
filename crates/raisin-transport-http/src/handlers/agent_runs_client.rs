// SPDX-License-Identifier: BSL-1.1

//! Client tools over HTTP: `POST /api/agent-runs/{repo}/{run}/client-result`.
//!
//! The answer a client posts for a tool call that ran on ITS side (a browser
//! page's WebMCP tool). A thin adapter over
//! `raisin_agent_runtime::api_client`: this layer authenticates the caller;
//! the rules (owner of the run, an open external request the client-tool
//! relay parked, expiry, idempotency by `delivery_id`) are core's.
//!
//! Answers `200 { status: "applied" | "duplicate" | "rejected", reason? }`;
//! a rejection (`request_not_open`, `not_a_client_call`, `forbidden`,
//! `expired`) is an answer, not an HTTP error. An unknown run is `404`, no
//! credentials `401`.

use axum::extract::{Path, Query};
use axum::{Extension, Json};
use raisin_agent_runtime::api_client::{self, ClientResultRequest};
use raisin_agent_runtime::ids::RunId;
use raisin_models::auth::AuthContext;
use serde_json::Value;

use super::agent_runs::{caller, host, map_err, scope_of, BranchQuery};
use crate::error::ApiError;
use crate::middleware::TenantInfo;

/// Deliver a client's answer to its tool call.
pub async fn client_result(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<ClientResultRequest>,
) -> Result<Json<Value>, ApiError> {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let answer = api_client::client_result(&*host()?, &scope, &RunId(run), &caller, req)
        .await
        .map_err(map_err)?;
    Ok(Json(serde_json::to_value(answer).unwrap_or(Value::Null)))
}
