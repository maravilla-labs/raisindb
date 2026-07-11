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

//! Token endpoint logic (RFC 6749 §4.1.3, OAuth 2.1 profile).
//!
//! Validates the `authorization_code` grant: authenticates the client, checks
//! the redirect URI matches the one bound to the code, verifies the PKCE
//! `code_verifier`, and yields a [`TokenGrant`] describing the access token to
//! mint. The actual JWT is signed by the resource server's existing JWT machinery
//! so the OAuth-issued token is verifiable on the same code path as a login token.

use super::error::{AuthServerError, AuthServerResult};
use super::model::{AuthorizationCode, OAuthClient, TokenEndpointAuthMethod};
use super::pkce::verify_pkce;
use super::registration::hash_client_secret;

/// The raw parameters of an `authorization_code` token request.
#[derive(Debug, Clone)]
pub struct AuthorizationCodeTokenRequest {
    /// `grant_type` — must be `authorization_code`.
    pub grant_type: String,
    /// `code` — the authorization code being redeemed.
    pub code: String,
    /// `redirect_uri` — must match the one bound to the code.
    pub redirect_uri: Option<String>,
    /// `client_id` — the redeeming client.
    pub client_id: String,
    /// `client_secret` — required for confidential clients.
    pub client_secret: Option<String>,
    /// `code_verifier` — the PKCE proof.
    pub code_verifier: Option<String>,
}

/// What the resource server needs to mint an access token for a redeemed code.
///
/// The HTTP layer feeds these into the JWT signer; the token's audience is the
/// MCP `resource` and its `scope` claim carries the consented scopes so the
/// resource server's scope gating and RLS line up.
#[derive(Debug, Clone)]
pub struct TokenGrant {
    /// Identity id (becomes the JWT `sub`).
    pub identity_id: String,
    /// Resource owner email.
    pub email: String,
    /// Tenant id.
    pub tenant_id: String,
    /// Repository the resource lives under.
    pub repository: String,
    /// Branch the resource lives under.
    pub branch: String,
    /// The token audience (the MCP server URL, RFC 8707).
    pub audience: String,
    /// The granted scopes (space-delimited).
    pub scope: String,
    /// The client the token was issued to.
    pub client_id: String,
}

/// Authenticate a client against its registration for a token request.
///
/// Public clients (no secret) are authenticated solely by the PKCE proof, so
/// only the `client_id` must match. Confidential clients must present a secret
/// whose hash matches the stored digest.
pub fn authenticate_client(
    client: &OAuthClient,
    presented_client_id: &str,
    presented_secret: Option<&str>,
) -> AuthServerResult<()> {
    if client.client_id != presented_client_id {
        return Err(AuthServerError::InvalidClient(
            "client_id mismatch".to_string(),
        ));
    }

    match client.token_endpoint_auth_method {
        TokenEndpointAuthMethod::None => {
            // Public client: a secret must NOT be presented.
            if presented_secret.is_some() {
                return Err(AuthServerError::InvalidClient(
                    "public client must not present a client_secret".to_string(),
                ));
            }
            Ok(())
        }
        TokenEndpointAuthMethod::ClientSecretPost | TokenEndpointAuthMethod::ClientSecretBasic => {
            let secret = presented_secret.ok_or_else(|| {
                AuthServerError::InvalidClient(
                    "confidential client must present a client_secret".to_string(),
                )
            })?;
            let expected = client.client_secret_hash.as_deref().ok_or_else(|| {
                AuthServerError::ServerError(
                    "confidential client has no stored secret hash".to_string(),
                )
            })?;
            if constant_time_str_eq(&hash_client_secret(secret), expected) {
                Ok(())
            } else {
                Err(AuthServerError::InvalidClient(
                    "client authentication failed".to_string(),
                ))
            }
        }
    }
}

/// Exchange a redeemed authorization code for a [`TokenGrant`].
///
/// The caller has already authenticated the client (via [`authenticate_client`])
/// and atomically removed the code from the store. This function performs the
/// remaining grant checks: grant type, code-binding (client + redirect URI),
/// expiry, and the PKCE verification (RFC 7636 §4.6).
pub fn exchange_authorization_code(
    req: &AuthorizationCodeTokenRequest,
    code: &AuthorizationCode,
    now: i64,
) -> AuthServerResult<TokenGrant> {
    if req.grant_type != "authorization_code" {
        return Err(AuthServerError::UnsupportedGrantType(format!(
            "expected authorization_code, got '{}'",
            req.grant_type
        )));
    }

    // The code must have been issued to this client.
    if code.client_id != req.client_id {
        return Err(AuthServerError::InvalidGrant(
            "authorization code was issued to a different client".to_string(),
        ));
    }

    // The redirect URI must match the one bound at authorization time.
    match &req.redirect_uri {
        Some(uri) if uri == &code.redirect_uri => {}
        Some(_) => {
            return Err(AuthServerError::InvalidGrant(
                "redirect_uri does not match the value from the authorization request".to_string(),
            ));
        }
        None => {
            return Err(AuthServerError::InvalidRequest(
                "redirect_uri is required for the authorization_code grant".to_string(),
            ));
        }
    }

    // Defense in depth: the store rejects expired codes, but re-check here.
    if code.is_expired(now) {
        return Err(AuthServerError::InvalidGrant(
            "authorization code has expired".to_string(),
        ));
    }

    // PKCE is mandatory: verify the presented verifier against the bound challenge.
    let verifier = req.code_verifier.as_deref().ok_or_else(|| {
        AuthServerError::InvalidRequest("code_verifier is required (PKCE)".to_string())
    })?;
    verify_pkce(code.code_challenge_method, &code.code_challenge, verifier)?;

    Ok(TokenGrant {
        identity_id: code.identity_id.clone(),
        email: code.email.clone(),
        tenant_id: code.tenant_id.clone(),
        repository: code.repository.clone(),
        branch: code.branch.clone(),
        audience: code.resource.clone(),
        scope: code.scope.clone(),
        client_id: code.client_id.clone(),
    })
}

/// Constant-time string comparison for secret-hash checks.
fn constant_time_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    let n = a.len().min(b.len());
    for i in 0..n {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authserver::pkce::CodeChallengeMethod;
    use crate::strategies::OidcStrategy;

    fn public_client() -> OAuthClient {
        OAuthClient {
            client_id: "client-1".to_string(),
            client_secret_hash: None,
            tenant_id: "tenant-a".to_string(),
            client_name: None,
            redirect_uris: vec!["http://127.0.0.1:9000/cb".to_string()],
            grant_types: vec!["authorization_code".to_string()],
            response_types: vec!["code".to_string()],
            scope: None,
            token_endpoint_auth_method: TokenEndpointAuthMethod::None,
            created_at: 0,
        }
    }

    fn code_for(challenge: &str, expires_at: i64) -> AuthorizationCode {
        AuthorizationCode {
            code: "the-code".to_string(),
            client_id: "client-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            redirect_uri: "http://127.0.0.1:9000/cb".to_string(),
            code_challenge: challenge.to_string(),
            code_challenge_method: CodeChallengeMethod::S256,
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            resource: "https://db.example.com/mcp/repo/main/srv".to_string(),
            scope: "reader".to_string(),
            expires_at,
        }
    }

    fn token_request(verifier: &str) -> AuthorizationCodeTokenRequest {
        AuthorizationCodeTokenRequest {
            grant_type: "authorization_code".to_string(),
            code: "the-code".to_string(),
            redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
            client_id: "client-1".to_string(),
            client_secret: None,
            code_verifier: Some(verifier.to_string()),
        }
    }

    #[test]
    fn full_exchange_succeeds_with_correct_verifier() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let client = public_client();
        let now = chrono::Utc::now().timestamp();

        authenticate_client(&client, "client-1", None).unwrap();
        let grant = exchange_authorization_code(
            &token_request(&verifier),
            &code_for(&challenge, now + 300),
            now,
        )
        .unwrap();

        assert_eq!(grant.identity_id, "id-1");
        assert_eq!(grant.audience, "https://db.example.com/mcp/repo/main/srv");
        assert_eq!(grant.scope, "reader");
    }

    #[test]
    fn exchange_fails_with_wrong_verifier() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let wrong = OidcStrategy::generate_code_verifier();
        let now = chrono::Utc::now().timestamp();

        let err = exchange_authorization_code(
            &token_request(&wrong),
            &code_for(&challenge, now + 300),
            now,
        )
        .expect_err("wrong verifier must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn exchange_fails_on_redirect_uri_mismatch() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let now = chrono::Utc::now().timestamp();
        let mut req = token_request(&verifier);
        req.redirect_uri = Some("http://127.0.0.1:9000/other".to_string());

        let err = exchange_authorization_code(&req, &code_for(&challenge, now + 300), now)
            .expect_err("redirect mismatch must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn exchange_fails_on_expired_code() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let now = chrono::Utc::now().timestamp();

        let err = exchange_authorization_code(
            &token_request(&verifier),
            &code_for(&challenge, now - 1),
            now,
        )
        .expect_err("expired code must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn public_client_rejects_presented_secret() {
        let err = authenticate_client(&public_client(), "client-1", Some("secret"))
            .expect_err("public client must not accept a secret");
        assert_eq!(err.code(), "invalid_client");
    }

    #[test]
    fn confidential_client_requires_matching_secret() {
        let mut client = public_client();
        client.token_endpoint_auth_method = TokenEndpointAuthMethod::ClientSecretPost;
        client.client_secret_hash = Some(hash_client_secret("s3cr3t"));

        authenticate_client(&client, "client-1", Some("s3cr3t")).expect("matching secret passes");
        let err = authenticate_client(&client, "client-1", Some("wrong"))
            .expect_err("wrong secret fails");
        assert_eq!(err.code(), "invalid_client");
    }
}
