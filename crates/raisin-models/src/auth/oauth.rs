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

//! Persisted state of the OAuth 2.1 authorization server.
//!
//! These types live here rather than in `raisin-auth` because they are
//! **replicated**: `raisin-replication` carries them in its operation log, and
//! it depends on `raisin-models` but not on `raisin-auth`. That is the same
//! reason [`Identity`](super::Identity), [`Session`](super::Session) and
//! `DatabaseAdminUser` live here. `raisin-auth` re-exports them, so the
//! protocol code still refers to them through `authserver::model`.
//!
//! The protocol logic itself — validation, PKCE, rotation — stays in
//! `raisin-auth`; only the data shapes are here.
//!
//! # Serialization
//!
//! Both structs carry `skip_serializing_if` fields, so any MessagePack
//! persistence MUST use the **named** encoding (`rmp_serde::to_vec_named`). In
//! the compact positional encoding a skipped field shifts every field after it,
//! and a `None` `client_secret_hash` comes back out as "invalid type: sequence,
//! expected a string".

use serde::{Deserialize, Serialize};

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
///
/// **These records must be durable and, in a cluster, replicated.** An MCP host
/// registers once via RFC 7591 and then caches the issued `client_id`
/// indefinitely; a client it cannot find is an `invalid_client` the user can
/// only escape by deleting and re-adding the connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// A persisted refresh token, stored as a **hash** of the value handed to the
/// client (the raw value is never written down, mirroring [`OAuthClient`]).
///
/// Tokens are chained into a *family*: every rotation issues a successor with
/// the same `family_id`. Presenting an already-consumed token means the chain
/// leaked, so the whole family is revoked (OAuth 2.1 §4.14.2 refresh-token
/// replay detection).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    ///
    /// This is the *durable* record of consumption. In a cluster it converges
    /// asynchronously, so it is a backstop rather than the primary guard — the
    /// authorization server marks consumption with a lock lease, which is
    /// immediate on every node.
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
