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

//! Authorization endpoint logic (RFC 6749 §4.1.1, OAuth 2.1 profile).
//!
//! This module is transport-agnostic: it validates the parameters of an
//! authorization request against a registered client and produces an
//! [`AuthorizationCode`] once the HTTP layer has authenticated the resource
//! owner. PKCE (`code_challenge` with `S256`) is mandatory.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

use super::error::{AuthServerError, AuthServerResult};
use super::model::{AuthorizationCode, OAuthClient};
use super::pkce::{validate_code_challenge, CodeChallengeMethod};
use super::scope::{check_requested_against_client, parse_scope};

/// Lifetime of an issued authorization code, in seconds (RFC 6749 §4.1.2
/// recommends ≤ 10 minutes; we use 5).
pub const AUTH_CODE_TTL_SECONDS: i64 = 300;

/// The validated parameters of an authorization request, ready to be turned
/// into a code once the resource owner is known.
///
/// Producing this value does **not** authenticate the user — it only proves the
/// request is well-formed and the client/redirect/PKCE/scope are acceptable.
#[derive(Debug, Clone)]
pub struct ValidatedAuthorizationRequest {
    /// The requesting client.
    pub client_id: String,
    /// The validated redirect URI (echoed back on the redirect).
    pub redirect_uri: String,
    /// The opaque client state to round-trip on the redirect.
    pub state: Option<String>,
    /// The PKCE challenge.
    pub code_challenge: String,
    /// The PKCE method.
    pub code_challenge_method: CodeChallengeMethod,
    /// The requested scopes (already checked against the client's set).
    pub requested_scopes: Vec<String>,
    /// The resource indicator (RFC 8707) — the MCP server URL the token targets.
    pub resource: String,
}

/// The raw parameters of an incoming authorization request.
#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    /// `response_type` — must be `code`.
    pub response_type: String,
    /// `client_id`.
    pub client_id: String,
    /// `redirect_uri`.
    pub redirect_uri: Option<String>,
    /// `state`.
    pub state: Option<String>,
    /// `code_challenge`.
    pub code_challenge: Option<String>,
    /// `code_challenge_method` (defaults to `S256` when a challenge is present).
    pub code_challenge_method: Option<String>,
    /// `scope`.
    pub scope: Option<String>,
    /// `resource` (RFC 8707) — the MCP server URL.
    pub resource: Option<String>,
}

/// Validate an authorization request against a registered client.
///
/// Errors that arise before the redirect URI is trusted ([`AuthServerError::InvalidClient`]
/// / [`AuthServerError::InvalidRedirectUri`]) signal the HTTP layer to render an
/// error page rather than redirect (RFC 6749 §4.1.2.1).
pub fn validate_authorization_request(
    req: &AuthorizationRequest,
    client: &OAuthClient,
) -> AuthServerResult<ValidatedAuthorizationRequest> {
    if req.response_type != "code" {
        return Err(AuthServerError::UnsupportedResponseType(format!(
            "only response_type=code is supported, got '{}'",
            req.response_type
        )));
    }

    if !client.allows_grant_type("authorization_code") {
        return Err(AuthServerError::UnauthorizedClient(
            "client is not registered for the authorization_code grant".to_string(),
        ));
    }

    // Resolve and validate the redirect URI. When the client registered exactly
    // one URI and the request omits it, that single URI is used (RFC 6749 §3.1.2.3).
    let redirect_uri = match &req.redirect_uri {
        Some(uri) => {
            if !client.allows_redirect_uri(uri) {
                return Err(AuthServerError::InvalidRedirectUri(format!(
                    "redirect_uri '{uri}' is not registered for this client"
                )));
            }
            uri.clone()
        }
        None => {
            if client.redirect_uris.len() == 1 {
                client.redirect_uris[0].clone()
            } else {
                return Err(AuthServerError::InvalidRedirectUri(
                    "redirect_uri is required when the client has multiple registered URIs"
                        .to_string(),
                ));
            }
        }
    };

    // PKCE is mandatory in OAuth 2.1.
    let code_challenge = req.code_challenge.clone().ok_or_else(|| {
        AuthServerError::InvalidRequest("code_challenge is required (PKCE)".to_string())
    })?;
    validate_code_challenge(&code_challenge)?;
    let code_challenge_method = match req.code_challenge_method.as_deref() {
        // An absent method defaults to S256 when a challenge is present.
        None => CodeChallengeMethod::S256,
        Some(m) => CodeChallengeMethod::parse(m)?,
    };

    // Scope: must be a subset of what the client registered (when restricted).
    let requested_scopes = req.scope.as_deref().map(parse_scope).unwrap_or_default();
    if let Err(bad) =
        check_requested_against_client(&requested_scopes, &client.registered_scopes())
    {
        return Err(AuthServerError::InvalidScope(format!(
            "scope '{bad}' is not permitted for this client"
        )));
    }

    // The resource indicator is required so the issued token can be audience-bound
    // to the specific MCP server (RFC 8707, MCP authorization profile).
    let resource = req.resource.clone().ok_or_else(|| {
        AuthServerError::InvalidRequest(
            "resource indicator is required to bind the token audience".to_string(),
        )
    })?;
    if url::Url::parse(&resource).is_err() {
        return Err(AuthServerError::InvalidRequest(format!(
            "resource '{resource}' is not a valid URI"
        )));
    }

    Ok(ValidatedAuthorizationRequest {
        client_id: client.client_id.clone(),
        redirect_uri,
        state: req.state.clone(),
        code_challenge,
        code_challenge_method,
        requested_scopes,
        resource,
    })
}

/// The authenticated resource owner an authorization code is issued for.
#[derive(Debug, Clone)]
pub struct ResourceOwner {
    /// The global identity id (JWT `sub`).
    pub identity_id: String,
    /// The resource owner's email.
    pub email: String,
    /// The repository the MCP resource lives under.
    pub repository: String,
    /// The branch the MCP resource lives under.
    pub branch: String,
    /// The scopes the resource owner consented to (after narrowing to grants).
    pub granted_scopes: Vec<String>,
}

/// Mint a single-use authorization code for a validated request and an
/// authenticated, consenting resource owner.
///
/// `now` is the current Unix timestamp. The returned code is opaque (256 bits of
/// randomness, base64url) and binds the client, redirect URI, PKCE challenge,
/// resource, and consented scopes for verification at the token endpoint.
pub fn issue_authorization_code(
    validated: &ValidatedAuthorizationRequest,
    owner: &ResourceOwner,
    tenant_id: &str,
    now: i64,
) -> AuthorizationCode {
    use rand::Rng;
    let random: [u8; 32] = rand::thread_rng().gen();
    let code = URL_SAFE_NO_PAD.encode(random);

    AuthorizationCode {
        code,
        client_id: validated.client_id.clone(),
        tenant_id: tenant_id.to_string(),
        redirect_uri: validated.redirect_uri.clone(),
        code_challenge: validated.code_challenge.clone(),
        code_challenge_method: validated.code_challenge_method,
        identity_id: owner.identity_id.clone(),
        email: owner.email.clone(),
        repository: owner.repository.clone(),
        branch: owner.branch.clone(),
        resource: validated.resource.clone(),
        scope: owner.granted_scopes.join(" "),
        expires_at: now + AUTH_CODE_TTL_SECONDS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authserver::model::TokenEndpointAuthMethod;
    use crate::strategies::OidcStrategy;

    fn client() -> OAuthClient {
        OAuthClient {
            client_id: "client-1".to_string(),
            client_secret_hash: None,
            tenant_id: "tenant-a".to_string(),
            client_name: None,
            redirect_uris: vec!["http://127.0.0.1:9000/cb".to_string()],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            scope: Some("reader editor".to_string()),
            token_endpoint_auth_method: TokenEndpointAuthMethod::None,
            created_at: 0,
        }
    }

    fn request(challenge: &str) -> AuthorizationRequest {
        AuthorizationRequest {
            response_type: "code".to_string(),
            client_id: "client-1".to_string(),
            redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
            state: Some("xyz".to_string()),
            code_challenge: Some(challenge.to_string()),
            code_challenge_method: Some("S256".to_string()),
            scope: Some("reader".to_string()),
            resource: Some("https://db.example.com/mcp/repo/main/srv".to_string()),
        }
    }

    #[test]
    fn valid_request_passes() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let validated = validate_authorization_request(&request(&challenge), &client()).unwrap();
        assert_eq!(validated.requested_scopes, vec!["reader".to_string()]);
        assert_eq!(validated.code_challenge_method, CodeChallengeMethod::S256);
    }

    #[test]
    fn missing_pkce_is_rejected() {
        let mut req = request("ignored");
        req.code_challenge = None;
        let err = validate_authorization_request(&req, &client()).expect_err("PKCE required");
        assert_eq!(err.code(), "invalid_request");
    }

    #[test]
    fn unregistered_redirect_is_rejected_and_not_redirectable() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let mut req = request(&challenge);
        req.redirect_uri = Some("http://127.0.0.1:9999/evil".to_string());
        let err = validate_authorization_request(&req, &client()).unwrap_err();
        assert!(!err.is_redirectable());
    }

    #[test]
    fn scope_outside_client_set_is_rejected() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let mut req = request(&challenge);
        req.scope = Some("reader admin".to_string());
        let err = validate_authorization_request(&req, &client()).unwrap_err();
        assert_eq!(err.code(), "invalid_scope");
    }

    #[test]
    fn missing_resource_is_rejected() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let mut req = request(&challenge);
        req.resource = None;
        let err = validate_authorization_request(&req, &client()).unwrap_err();
        assert_eq!(err.code(), "invalid_request");
    }

    #[test]
    fn issued_code_binds_request_and_owner() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let validated = validate_authorization_request(&request(&challenge), &client()).unwrap();
        let owner = ResourceOwner {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            granted_scopes: vec!["reader".to_string()],
        };
        let code = issue_authorization_code(&validated, &owner, "tenant-a", 1000);
        assert_eq!(code.code_challenge, challenge);
        assert_eq!(code.identity_id, "id-1");
        assert_eq!(code.scope, "reader");
        assert_eq!(code.expires_at, 1000 + AUTH_CODE_TTL_SECONDS);
        assert!(!code.code.is_empty());
    }
}
