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

//! Data types for the OAuth 2.1 authorization server: registered clients,
//! issued authorization codes, and the request/response shapes exchanged at the
//! `/authorize`, `/token`, and `/register` endpoints.

use serde::{Deserialize, Serialize};

use super::pkce::CodeChallengeMethod;

/// How a client authenticates at the token endpoint (RFC 7591 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenEndpointAuthMethod {
    /// Public client — no secret. The only credential is the PKCE proof.
    /// This is the default for interactive MCP clients.
    None,
    /// Confidential client presenting `client_secret` in the POST body.
    ClientSecretPost,
    /// Confidential client presenting `client_secret` via HTTP Basic auth.
    ClientSecretBasic,
}

impl TokenEndpointAuthMethod {
    /// Whether the client is confidential (holds a secret).
    pub fn is_confidential(&self) -> bool {
        matches!(self, Self::ClientSecretPost | Self::ClientSecretBasic)
    }
}

/// A client registered with the authorization server.
///
/// `client_secret_hash` is `Some` only for confidential clients and holds a
/// SHA-256 hex digest of the secret — the raw secret is returned exactly once
/// at registration and never stored, mirroring the API-key store's design.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClient {
    /// Stable public client identifier.
    pub client_id: String,
    /// SHA-256 hex digest of the client secret (confidential clients only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_hash: Option<String>,
    /// Tenant this client belongs to.
    pub tenant_id: String,
    /// Human-readable client name for consent UIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    /// Allowed redirect URIs. The authorization endpoint requires an exact match.
    pub redirect_uris: Vec<String>,
    /// Grant types the client may use (e.g. `authorization_code`, `refresh_token`).
    pub grant_types: Vec<String>,
    /// Response types the client may use (e.g. `code`).
    pub response_types: Vec<String>,
    /// Space-delimited set of scopes the client may request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Token-endpoint authentication method.
    pub token_endpoint_auth_method: TokenEndpointAuthMethod,
    /// Issuance time (Unix seconds).
    pub created_at: i64,
}

impl OAuthClient {
    /// Whether `uri` exactly matches one of the registered redirect URIs.
    pub fn allows_redirect_uri(&self, uri: &str) -> bool {
        self.redirect_uris.iter().any(|u| u == uri)
    }

    /// Whether this client is permitted to use `grant_type`.
    pub fn allows_grant_type(&self, grant_type: &str) -> bool {
        self.grant_types.iter().any(|g| g == grant_type)
    }

    /// The scopes the client is registered to request, as a slice of tokens.
    pub fn registered_scopes(&self) -> Vec<String> {
        self.scope
            .as_deref()
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

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

/// A persisted refresh token, stored as a **hash** of the value handed to the
/// client (the raw value is never written down, mirroring `OAuthClient`).
///
/// Tokens are chained into a *family*: every rotation issues a successor with
/// the same `family_id`. Presenting an already-consumed token means the chain
/// leaked, so the whole family is revoked (OAuth 2.1 §4.14.2 refresh-token
/// replay detection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshToken {
    /// SHA-256 hex digest of the opaque token value.
    pub token_hash: String,
    /// Identifier shared by every token in this rotation chain.
    pub family_id: String,
    /// Client the token was issued to.
    pub client_id: String,
    /// Tenant context.
    pub tenant_id: String,
    /// The resource owner's identity id.
    pub identity_id: String,
    /// The resource owner's email.
    pub email: String,
    /// The repository the resource lives under.
    pub repository: String,
    /// The branch the resource lives under.
    pub branch: String,
    /// The resource indicator the refreshed access token will target.
    pub resource: String,
    /// The scopes this token may refresh (space-delimited).
    pub scope: String,
    /// Issuance time (Unix seconds).
    pub issued_at: i64,
    /// Expiry (Unix seconds).
    pub expires_at: i64,
    /// When the token was redeemed, if it has been. A second presentation of a
    /// consumed token is a replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_at: Option<i64>,
}

impl RefreshToken {
    /// Whether the token has passed its expiry instant.
    pub fn is_expired(&self, now: i64) -> bool {
        now >= self.expires_at
    }

    /// Whether the token has already been redeemed.
    pub fn is_consumed(&self) -> bool {
        self.consumed_at.is_some()
    }

    /// The scopes this token may refresh, as a slice of tokens.
    pub fn scopes(&self) -> Vec<&str> {
        self.scope.split_whitespace().collect()
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
