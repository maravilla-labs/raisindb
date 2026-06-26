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
use axum::Json;

use super::helpers::issuer_from_request;
use crate::error::ApiError;
use crate::state::AppState;

/// `GET /.well-known/oauth-authorization-server`.
///
/// Serves RFC 8414 authorization-server metadata for the issuer derived from the
/// request, advertising only the grants and PKCE method RaisinDB implements.
#[cfg(feature = "storage-rocksdb")]
pub async fn authorization_server_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<raisin_auth::authserver::AuthorizationServerMetadata>, ApiError> {
    let issuer = issuer_from_request(&headers);
    Ok(Json(state.oauth_server.metadata(&issuer)))
}

/// `GET /.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}`.
///
/// Serves RFC 9728 protected-resource metadata for a single MCP server. The
/// `resource` is the canonical MCP endpoint URL (`{issuer}/mcp/{repo}/{branch}/{slug}`),
/// and the sole authorization server is this issuer.
#[cfg(feature = "storage-rocksdb")]
pub async fn protected_resource_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((repo, branch, slug)): Path<(String, String, String)>,
) -> Result<Json<raisin_auth::authserver::ProtectedResourceMetadata>, ApiError> {
    let issuer = issuer_from_request(&headers);
    let resource = format!("{issuer}/mcp/{repo}/{branch}/{slug}");
    Ok(Json(
        state
            .oauth_server
            .protected_resource_metadata(&resource, &issuer),
    ))
}
