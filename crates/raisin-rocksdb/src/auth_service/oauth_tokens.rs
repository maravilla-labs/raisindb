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

//! Resource-bound access tokens for the OAuth 2.1 authorization server.
//!
//! These reuse the same HS256 signing key as login tokens, but are **audience
//! pinned**: a resource server validates them with
//! [`validate_resource_token`](AuthService::validate_resource_token), which
//! requires `aud` to equal the resource being accessed. The login path
//! [`validate_user_token`](AuthService::validate_user_token) deliberately rejects
//! them, so a token consented for one MCP resource can never act as a general
//! login token elsewhere (RFC 8707). They differ from a login token in two
//! claims: `aud` is set to the MCP resource URL and `scope` carries the consented
//! scopes, letting a resource server gate on the token's scopes without
//! re-deriving them.

use super::AuthService;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use raisin_auth::authserver::TokenGrant;
use raisin_error::{Error, Result};
use raisin_models::auth::{AuthClaims, GlobalFlags, TokenType};

/// Lifetime of an OAuth-issued access token, in seconds (one hour, matching the
/// login access-token lifetime).
const OAUTH_ACCESS_TOKEN_TTL_SECONDS: u64 = 3600;

impl AuthService {
    /// Mint a resource-bound JWT access token from an OAuth [`TokenGrant`].
    ///
    /// Returns the signed compact JWT and its lifetime in seconds. The token is
    /// an `Access` token whose `aud` is the grant's audience and whose `scope`
    /// is the consented scope string. It is signed with the same key as login
    /// tokens, so the existing user-token validator accepts it.
    pub fn mint_oauth_access_token(&self, grant: &TokenGrant) -> Result<(String, u64)> {
        let now = Utc::now();
        let expiry = now + Duration::seconds(OAUTH_ACCESS_TOKEN_TTL_SECONDS as i64);

        let scope = if grant.scope.is_empty() {
            None
        } else {
            Some(grant.scope.clone())
        };

        let claims = AuthClaims {
            sub: grant.identity_id.clone(),
            email: grant.email.clone(),
            tenant_id: grant.tenant_id.clone(),
            // The repository the resource lives under, so request handlers that
            // read `repository` from the token stay scoped to the right repo.
            repository: Some(grant.repository.clone()),
            home: None,
            // OAuth tokens are not tied to an interactive session; the session id
            // namespaces the jti-derived cache key by the issuing client instead.
            sid: format!("oauth:{}", grant.client_id),
            auth_strategy: "oauth2".to_string(),
            auth_time: now.timestamp(),
            global_flags: GlobalFlags::default(),
            token_type: TokenType::Access,
            exp: expiry.timestamp(),
            iat: now.timestamp(),
            nbf: Some(now.timestamp()),
            jti: uuid::Uuid::new_v4().to_string(),
            iss: Some("raisindb".to_string()),
            aud: Some(grant.audience.clone()),
            scope,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|e| Error::Backend(format!("Failed to mint OAuth access token: {e}")))?;

        Ok((token, OAUTH_ACCESS_TOKEN_TTL_SECONDS))
    }

    /// Validate a resource-bound access token, enforcing the expected audience.
    ///
    /// Unlike [`validate_user_token`](AuthService::validate_user_token) — which
    /// skips audience checks so both login and OAuth tokens share one path — this
    /// requires the token's `aud` to equal `expected_audience`, the check a
    /// resource server performs to reject a token minted for a different MCP
    /// resource (RFC 8707).
    pub fn validate_resource_token(
        &self,
        token: &str,
        expected_audience: &str,
    ) -> Result<AuthClaims> {
        let mut validation = Validation::default();
        validation.set_audience(&[expected_audience]);

        let token_data = decode::<AuthClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &validation,
        )
        .map_err(|e| Error::Unauthorized(format!("Invalid resource token: {e}")))?;

        let claims = token_data.claims;
        if !matches!(
            claims.token_type,
            TokenType::Access | TokenType::Impersonation { .. }
        ) {
            return Err(Error::Unauthorized(
                "Expected access token, got different token type".to_string(),
            ));
        }
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdminUserStore, RocksDBConfig, RocksDBStorage};

    fn service() -> (tempfile::TempDir, AuthService) {
        // A temp RocksDB backs the admin store; these methods only touch the
        // JWT key, but the store needs a real db handle.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config = RocksDBConfig::default().with_path(temp_dir.path());
        let storage = RocksDBStorage::with_config(config).unwrap();
        let store = AdminUserStore::new(storage.db().clone());
        (temp_dir, AuthService::new(store, "test-secret".to_string()))
    }

    fn grant() -> TokenGrant {
        TokenGrant {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            tenant_id: "tenant-a".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            audience: "https://db.example.com/mcp/repo/main/srv".to_string(),
            scope: "reader staff".to_string(),
            client_id: "client-1".to_string(),
        }
    }

    #[test]
    fn minted_token_carries_audience_and_scope() {
        let (_dir, svc) = service();
        let (token, ttl) = svc.mint_oauth_access_token(&grant()).unwrap();
        assert_eq!(ttl, OAUTH_ACCESS_TOKEN_TTL_SECONDS);

        // Validated as a resource token (the only path that accepts it), the
        // consented scope and bound audience are present.
        let claims = svc
            .validate_resource_token(&token, "https://db.example.com/mcp/repo/main/srv")
            .unwrap();
        assert_eq!(claims.sub, "id-1");
        assert_eq!(claims.scope.as_deref(), Some("reader staff"));
        assert_eq!(
            claims.aud.as_deref(),
            Some("https://db.example.com/mcp/repo/main/srv")
        );
    }

    #[test]
    fn resource_token_is_rejected_as_login_token() {
        let (_dir, svc) = service();
        let (token, _) = svc.mint_oauth_access_token(&grant()).unwrap();

        // The login path must refuse an audience-pinned token so it cannot be
        // replayed as a general user token on non-resource endpoints.
        let err = svc
            .validate_user_token(&token)
            .expect_err("aud-bearing token rejected by login path");
        assert!(matches!(err, Error::Unauthorized(_)));
    }

    #[test]
    fn resource_validation_enforces_audience() {
        let (_dir, svc) = service();
        let (token, _) = svc.mint_oauth_access_token(&grant()).unwrap();

        svc.validate_resource_token(&token, "https://db.example.com/mcp/repo/main/srv")
            .expect("correct audience accepted");

        let err = svc
            .validate_resource_token(&token, "https://db.example.com/mcp/other/main/srv")
            .expect_err("wrong audience rejected");
        assert!(matches!(err, Error::Unauthorized(_)));
    }
}
