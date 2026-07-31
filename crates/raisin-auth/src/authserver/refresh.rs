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

//! The `refresh_token` grant (RFC 6749 §6, OAuth 2.1 profile).
//!
//! Without this grant an MCP client must send the resource owner back through
//! the full `/authorize` + consent round trip every time the one-hour access
//! token expires, which reads to the user as "the connector keeps logging me
//! out".
//!
//! # Rotation is mandatory
//!
//! OAuth 2.1 requires that a public client's refresh token be **rotated** on
//! every use: redeeming a token consumes it and issues a successor. The
//! successor carries the same `family_id`, so a token that is presented twice
//! proves the chain leaked — the server then revokes the entire family rather
//! than the single token, because it cannot tell the attacker's copy from the
//! legitimate client's. That is why [`super::store::RefreshTokenStore`] marks
//! tokens consumed instead of deleting them: a deleted token is
//! indistinguishable from one that never existed, and replay detection needs
//! the difference.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

use super::error::{AuthServerError, AuthServerResult};
use super::model::{OAuthClient, RefreshToken};
use super::registration::sha256_hex;
use super::token::TokenGrant;

/// Lifetime of an issued refresh token (30 days).
///
/// Rotation resets it, so an actively used connector never expires; one left
/// idle past this window falls back to a full authorization round trip.
pub const REFRESH_TOKEN_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// The raw parameters of a `refresh_token` token request.
#[derive(Debug, Clone)]
pub struct RefreshTokenRequest {
    /// `grant_type` — must be `refresh_token`.
    pub grant_type: String,
    /// `refresh_token` — the opaque value previously issued.
    pub refresh_token: String,
    /// `client_id` — the refreshing client.
    pub client_id: String,
    /// `client_secret` — required for confidential clients.
    pub client_secret: Option<String>,
    /// `scope` — an optional *narrowing* of the originally granted scopes.
    pub scope: Option<String>,
}

/// A freshly minted refresh token.
///
/// `value` is handed to the client exactly once; `record` (which holds only the
/// hash) is what gets persisted.
#[derive(Debug, Clone)]
pub struct IssuedRefreshToken {
    /// The opaque token value to return to the client.
    pub value: String,
    /// The record to persist.
    pub record: RefreshToken,
}

/// SHA-256 hex digest of a refresh-token value, as stored.
pub fn hash_refresh_token(value: &str) -> String {
    sha256_hex(value)
}

/// Generate a new opaque refresh-token value.
fn generate_refresh_value() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    format!("rt_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Start a new rotation chain.
pub fn new_family_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Mint a refresh token for `grant`, joining the rotation chain `family_id`.
pub fn issue_refresh_token(grant: &TokenGrant, family_id: String, now: i64) -> IssuedRefreshToken {
    let value = generate_refresh_value();
    let record = RefreshToken {
        token_hash: hash_refresh_token(&value),
        family_id,
        client_id: grant.client_id.clone(),
        tenant_id: grant.tenant_id.clone(),
        identity_id: grant.identity_id.clone(),
        email: grant.email.clone(),
        repository: grant.repository.clone(),
        branch: grant.branch.clone(),
        resource: grant.audience.clone(),
        scope: grant.scope.clone(),
        issued_at: now,
        expires_at: now + REFRESH_TOKEN_TTL_SECONDS,
        consumed_at: None,
    };
    IssuedRefreshToken { value, record }
}

/// Validate a refresh request against the record it named and yield the grant
/// for the replacement access token.
///
/// The caller has already authenticated the client and atomically consumed the
/// record; replay detection (a record that was *already* consumed) is handled
/// upstream because it must revoke the whole family, which needs store access.
///
/// A `scope` parameter may only **narrow** the original grant (RFC 6749 §6);
/// widening it here would let a leaked token escalate beyond what the resource
/// owner consented to.
pub fn exchange_refresh_token(
    req: &RefreshTokenRequest,
    record: &RefreshToken,
    client: &OAuthClient,
    now: i64,
) -> AuthServerResult<TokenGrant> {
    if req.grant_type != "refresh_token" {
        return Err(AuthServerError::UnsupportedGrantType(format!(
            "expected refresh_token, got '{}'",
            req.grant_type
        )));
    }

    if !client.allows_grant_type("refresh_token") {
        return Err(AuthServerError::UnauthorizedClient(
            "client is not registered for the refresh_token grant".to_string(),
        ));
    }

    // The token must have been issued to this client.
    if record.client_id != req.client_id {
        return Err(AuthServerError::InvalidGrant(
            "refresh token was issued to a different client".to_string(),
        ));
    }

    if record.is_expired(now) {
        return Err(AuthServerError::InvalidGrant(
            "refresh token has expired".to_string(),
        ));
    }

    let scope = match req.scope.as_deref() {
        None => record.scope.clone(),
        Some(requested) => narrow_scope(requested, record)?,
    };

    Ok(TokenGrant {
        identity_id: record.identity_id.clone(),
        email: record.email.clone(),
        tenant_id: record.tenant_id.clone(),
        repository: record.repository.clone(),
        branch: record.branch.clone(),
        audience: record.resource.clone(),
        scope,
        client_id: record.client_id.clone(),
    })
}

/// Resolve a requested scope against the granted one, rejecting any token that
/// was not part of the original grant.
fn narrow_scope(requested: &str, record: &RefreshToken) -> AuthServerResult<String> {
    let granted = record.scopes();
    let mut kept = Vec::new();
    for token in requested.split_whitespace() {
        if !granted.contains(&token) {
            return Err(AuthServerError::InvalidScope(format!(
                "scope '{token}' was not part of the original grant"
            )));
        }
        kept.push(token);
    }
    Ok(kept.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authserver::model::TokenEndpointAuthMethod;

    fn client() -> OAuthClient {
        OAuthClient {
            client_id: "client-1".to_string(),
            client_secret_hash: None,
            tenant_id: "tenant-a".to_string(),
            client_name: None,
            redirect_uris: vec!["https://app.example.com/cb".to_string()],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            scope: None,
            token_endpoint_auth_method: TokenEndpointAuthMethod::None,
            created_at: 0,
        }
    }

    fn grant() -> TokenGrant {
        TokenGrant {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            tenant_id: "tenant-a".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            audience: "https://db.example.com/mcp/repo/main/srv".to_string(),
            scope: "reader writer".to_string(),
            client_id: "client-1".to_string(),
        }
    }

    fn request() -> RefreshTokenRequest {
        RefreshTokenRequest {
            grant_type: "refresh_token".to_string(),
            refresh_token: "rt_whatever".to_string(),
            client_id: "client-1".to_string(),
            client_secret: None,
            scope: None,
        }
    }

    #[test]
    fn issued_token_stores_only_the_hash() {
        let issued = issue_refresh_token(&grant(), new_family_id(), 1000);
        assert!(issued.value.starts_with("rt_"));
        assert_eq!(issued.record.token_hash, hash_refresh_token(&issued.value));
        assert!(!issued.record.token_hash.contains(&issued.value));
        assert_eq!(issued.record.expires_at, 1000 + REFRESH_TOKEN_TTL_SECONDS);
    }

    #[test]
    fn exchange_carries_the_original_grant() {
        let issued = issue_refresh_token(&grant(), new_family_id(), 1000);
        let out = exchange_refresh_token(&request(), &issued.record, &client(), 1001).unwrap();
        assert_eq!(out.identity_id, "id-1");
        assert_eq!(out.audience, "https://db.example.com/mcp/repo/main/srv");
        assert_eq!(out.scope, "reader writer");
    }

    #[test]
    fn scope_may_be_narrowed_but_not_widened() {
        let issued = issue_refresh_token(&grant(), new_family_id(), 1000);

        let mut narrowing = request();
        narrowing.scope = Some("reader".to_string());
        let out = exchange_refresh_token(&narrowing, &issued.record, &client(), 1001).unwrap();
        assert_eq!(out.scope, "reader");

        let mut widening = request();
        widening.scope = Some("reader admin".to_string());
        let err = exchange_refresh_token(&widening, &issued.record, &client(), 1001)
            .expect_err("widening must be rejected");
        assert_eq!(err.code(), "invalid_scope");
    }

    #[test]
    fn expired_token_is_rejected() {
        let issued = issue_refresh_token(&grant(), new_family_id(), 1000);
        let err = exchange_refresh_token(
            &request(),
            &issued.record,
            &client(),
            1000 + REFRESH_TOKEN_TTL_SECONDS,
        )
        .expect_err("expired token must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn client_without_the_grant_is_rejected() {
        let issued = issue_refresh_token(&grant(), new_family_id(), 1000);
        let mut c = client();
        c.grant_types = vec!["authorization_code".to_string()];
        let err = exchange_refresh_token(&request(), &issued.record, &c, 1001)
            .expect_err("unregistered grant must fail");
        assert_eq!(err.code(), "unauthorized_client");
    }

    #[test]
    fn token_from_another_client_is_rejected() {
        let issued = issue_refresh_token(&grant(), new_family_id(), 1000);
        let mut req = request();
        req.client_id = "client-2".to_string();
        let err = exchange_refresh_token(&req, &issued.record, &client(), 1001)
            .expect_err("cross-client refresh must fail");
        assert_eq!(err.code(), "invalid_grant");
    }
}
