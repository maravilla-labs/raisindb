// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Discovery metadata endpoints.
//!
//! - `GET /.well-known/oauth-authorization-server` — RFC 8414 metadata that an
//!   MCP client uses to locate the `/authorize`, `/token`, and `/register`
//!   endpoints and learn the server only accepts PKCE `S256`.
//! - `GET /.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}` —
//!   RFC 9728 metadata, path-scoped to one MCP resource, naming this server as
//!   the authorization server that guards it.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use super::helpers::{
    issuer_from_request, load_tenant_trusted_hosts, mcp_resource_url, oauth_error_response,
};
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// `GET /.well-known/oauth-authorization-server`.
///
/// Serves RFC 8414 authorization-server metadata for the issuer derived from the
/// request, advertising only the grants and PKCE method RaisinDB implements. The
/// issuer host is validated against the tenant's trusted-host allowlist.
#[cfg(feature = "storage-rocksdb")]
pub async fn authorization_server_metadata(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    headers: HeaderMap,
) -> Response {
    let tenant_hosts = load_tenant_trusted_hosts(&state, &tenant_info.tenant_id).await;
    let issuer = match issuer_from_request(&headers, &tenant_hosts) {
        Ok(issuer) => issuer,
        Err(err) => return oauth_error_response(&err),
    };
    Json(state.oauth_server.metadata(&issuer)).into_response()
}

/// `GET /.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}`.
///
/// Serves RFC 9728 protected-resource metadata for a single MCP server. The
/// `resource` is the canonical MCP endpoint URL (`{issuer}/mcp/{repo}/{branch}/{slug}`),
/// and the sole authorization server is this issuer.
#[cfg(feature = "storage-rocksdb")]
pub async fn protected_resource_metadata(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    headers: HeaderMap,
    Path((repo, branch, slug)): Path<(String, String, String)>,
) -> Response {
    let tenant_hosts = load_tenant_trusted_hosts(&state, &tenant_info.tenant_id).await;
    let issuer = match issuer_from_request(&headers, &tenant_hosts) {
        Ok(issuer) => issuer,
        Err(err) => return oauth_error_response(&err),
    };
    let resource = mcp_resource_url(&issuer, &repo, &branch, &slug);
    Json(
        state
            .oauth_server
            .protected_resource_metadata(&resource, &issuer),
    )
    .into_response()
}
