// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Data types for the OAuth 2.1 authorization server: issued authorization
//! codes and the request/response shapes exchanged at the `/authorize`,
//! `/token`, and `/register` endpoints.
//!
//! The **persisted** types — [`OAuthClient`], [`RefreshToken`] and
//! [`TokenEndpointAuthMethod`] — live in `raisin-models` and are re-exported
//! here, because `raisin-replication` carries them in its operation log and
//! depends on `raisin-models` but not on this crate. Everything defined
//! directly below is transport or in-flight state that is never replicated.

use serde::{Deserialize, Serialize};

use super::pkce::CodeChallengeMethod;

pub use raisin_models::auth::{OAuthClient, RefreshToken, TokenEndpointAuthMethod};

/// A single-use authorization code bound to the consenting resource owner and
/// the PKCE challenge the client must later prove.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCode {
    /// The opaque code value handed to the client.
    pub code: String,
    /// Client the code was issued to.
    pub client_id: String,
    /// Tenant context.
    pub tenant_id: String,
    /// Redirect URI presented at the authorization request (must match at exchange).
    pub redirect_uri: String,
    /// PKCE challenge captured at authorization (verified at exchange).
    pub code_challenge: String,
    /// PKCE transform used for the challenge.
    pub code_challenge_method: CodeChallengeMethod,
    /// The authenticated resource owner's identity id.
    pub identity_id: String,
    /// The resource owner's email (carried into the minted token).
    pub email: String,
    /// The repository the resource (MCP server) lives under.
    pub repository: String,
    /// The branch the resource (MCP server) lives under.
    pub branch: String,
    /// The resource indicator (RFC 8707) — the MCP server URL the token targets.
    pub resource: String,
    /// The scopes the resource owner consented to (space-delimited).
    pub scope: String,
    /// Expiry (Unix seconds). Codes are short-lived (RFC 6749 §4.1.2 recommends ≤10 min).
    pub expires_at: i64,
}

impl AuthorizationCode {
    /// Whether the code has passed its expiry instant.
    pub fn is_expired(&self, now: i64) -> bool {
        now >= self.expires_at
    }
}

/// The successful token-endpoint response (RFC 6749 §5.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    /// The minted JWT access token.
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: String,
    /// Access-token lifetime in seconds.
    pub expires_in: u64,
    /// The refresh token, when a refresh grant is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// The space-delimited scopes actually granted.
    pub scope: String,
}

/// The dynamic client registration request body (RFC 7591 §2).
///
/// All fields are optional in the wire format; defaults are applied during
/// registration so an MCP client can register with just `redirect_uris`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientRegistrationRequest {
    /// Requested redirect URIs.
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    /// Requested token-endpoint auth method.
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
    /// Requested grant types.
    #[serde(default)]
    pub grant_types: Vec<String>,
    /// Requested response types.
    #[serde(default)]
    pub response_types: Vec<String>,
    /// Human-readable client name.
    #[serde(default)]
    pub client_name: Option<String>,
    /// Requested scope (space-delimited).
    #[serde(default)]
    pub scope: Option<String>,
}

/// The dynamic client registration response body (RFC 7591 §3.2.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRegistrationResponse {
    /// The issued client identifier.
    pub client_id: String,
    /// The issued client secret (confidential clients only). Returned once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Issuance time for the client id (Unix seconds).
    pub client_id_issued_at: i64,
    /// Registered redirect URIs (echoed).
    pub redirect_uris: Vec<String>,
    /// Granted token-endpoint auth method.
    pub token_endpoint_auth_method: TokenEndpointAuthMethod,
    /// Granted grant types.
    pub grant_types: Vec<String>,
    /// Granted response types.
    pub response_types: Vec<String>,
    /// Client name (echoed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    /// Granted scope (space-delimited).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}
