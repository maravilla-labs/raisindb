// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dynamic Client Registration endpoint (RFC 7591): `POST /register`.
//!
//! An interactive MCP client with no pre-provisioned credentials registers
//! itself here, supplying its redirect URIs and receiving a `client_id` (and,
//! for confidential clients, a one-time `client_secret`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use super::helpers::oauth_error_response;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// `POST /register` — register a new OAuth client for the request's tenant.
///
/// Returns `201 Created` with the RFC 7591 registration response on success, or
/// an RFC 7591 §3.2.2 error body on rejection.
#[cfg(feature = "storage-rocksdb")]
pub async fn register_client(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<raisin_auth::authserver::ClientRegistrationRequest>,
) -> Response {
    match state
        .oauth_server
        .register_client(&tenant_info.tenant_id, req)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(err) => oauth_error_response(&err),
    }
}
