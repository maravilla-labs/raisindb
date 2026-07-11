// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Token endpoint (RFC 6749 §4.1.3, OAuth 2.1 profile): `POST /token`.
//!
//! Accepts the `application/x-www-form-urlencoded` `authorization_code` grant,
//! authenticates the client, verifies the PKCE `code_verifier`, and mints a
//! resource-bound JWT access token (audience = the MCP resource, `scope` = the
//! consented scopes) using the resource server's existing JWT machinery.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form, Json};
use serde::Deserialize;

use raisin_auth::authserver::{AuthServerError, AuthorizationCodeTokenRequest, TokenResponse};

use super::helpers::oauth_error_response;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// The `POST /token` form body.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenForm {
    /// The grant type — only `authorization_code` is supported here.
    pub grant_type: String,
    /// The authorization code to redeem.
    pub code: Option<String>,
    /// The redirect URI bound to the code.
    pub redirect_uri: Option<String>,
    /// The client id (also accepted via HTTP Basic auth).
    pub client_id: Option<String>,
    /// The client secret (confidential clients; also via HTTP Basic auth).
    pub client_secret: Option<String>,
    /// The PKCE verifier.
    pub code_verifier: Option<String>,
}

/// `POST /token` — redeem an authorization code for an access token.
#[cfg(feature = "storage-rocksdb")]
pub async fn token(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    // Client credentials may arrive in the body or via HTTP Basic auth.
    let (client_id, client_secret) = match resolve_client_credentials(&headers, &form) {
        Ok(creds) => creds,
        Err(err) => return oauth_error_response(&err),
    };

    let code = match form.code.clone() {
        Some(c) => c,
        None => {
            return oauth_error_response(&AuthServerError::InvalidRequest(
                "code is required".to_string(),
            ))
        }
    };

    let req = AuthorizationCodeTokenRequest {
        grant_type: form.grant_type.clone(),
        code,
        redirect_uri: form.redirect_uri.clone(),
        client_id,
        client_secret,
        code_verifier: form.code_verifier.clone(),
    };

    // Redeem the code: load + authenticate client, consume code, verify PKCE.
    let grant = match state
        .oauth_server
        .redeem_authorization_code(&tenant_info.tenant_id, &req)
        .await
    {
        Ok(grant) => grant,
        Err(err) => return oauth_error_response(&err),
    };

    // Mint the resource-bound JWT with the existing signing machinery.
    let auth_service = match state.auth_service() {
        Some(svc) => svc,
        None => {
            return oauth_error_response(&AuthServerError::ServerError(
                "authentication service is not configured".to_string(),
            ))
        }
    };
    let (access_token, expires_in) = match auth_service.mint_oauth_access_token(&grant) {
        Ok(token) => token,
        Err(e) => {
            return oauth_error_response(&AuthServerError::ServerError(format!(
                "failed to mint access token: {e}"
            )))
        }
    };

    let response = TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in,
        refresh_token: None,
        scope: grant.scope,
    };

    // Token responses must not be cached (RFC 6749 §5.1).
    (
        [
            (axum::http::header::CACHE_CONTROL, "no-store"),
            (axum::http::header::PRAGMA, "no-cache"),
        ],
        Json(response),
    )
        .into_response()
}

/// Resolve the client id + secret from either an HTTP Basic `Authorization`
/// header (`client_secret_basic`) or the form body (`client_secret_post` / a
/// public client). A `client_id` is mandatory in one of the two places.
fn resolve_client_credentials(
    headers: &HeaderMap,
    form: &TokenForm,
) -> Result<(String, Option<String>), AuthServerError> {
    if let Some((id, secret)) = parse_basic_auth(headers)? {
        return Ok((id, Some(secret)));
    }
    match &form.client_id {
        Some(id) => Ok((id.clone(), form.client_secret.clone())),
        None => Err(AuthServerError::InvalidClient(
            "client_id is required (in the body or via HTTP Basic auth)".to_string(),
        )),
    }
}

/// Parse an HTTP Basic `Authorization` header into `(client_id, client_secret)`.
///
/// Returns `Ok(None)` when no Basic header is present, and an error when the
/// header is present but malformed.
fn parse_basic_auth(headers: &HeaderMap) -> Result<Option<(String, String)>, AuthServerError> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(None);
    };
    let Some(b64) = value.strip_prefix("Basic ") else {
        // A non-Basic scheme (e.g. Bearer) is not client authentication here.
        return Ok(None);
    };

    let decoded = STANDARD
        .decode(b64.trim())
        .map_err(|_| AuthServerError::InvalidClient("malformed Basic credentials".to_string()))?;
    let text = String::from_utf8(decoded).map_err(|_| {
        AuthServerError::InvalidClient("Basic credentials are not UTF-8".to_string())
    })?;
    let (id, secret) = text.split_once(':').ok_or_else(|| {
        AuthServerError::InvalidClient("Basic credentials must be id:secret".to_string())
    })?;
    Ok(Some((id.to_string(), secret.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::AUTHORIZATION;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test]
    fn parses_basic_auth_header() {
        let mut headers = HeaderMap::new();
        let creds = STANDARD.encode("client-1:s3cr3t");
        headers.insert(AUTHORIZATION, format!("Basic {creds}").parse().unwrap());
        let parsed = parse_basic_auth(&headers).unwrap().unwrap();
        assert_eq!(parsed, ("client-1".to_string(), "s3cr3t".to_string()));
    }

    #[test]
    fn body_credentials_used_when_no_basic_header() {
        let headers = HeaderMap::new();
        let form = TokenForm {
            grant_type: "authorization_code".to_string(),
            code: Some("c".to_string()),
            redirect_uri: None,
            client_id: Some("client-1".to_string()),
            client_secret: None,
            code_verifier: None,
        };
        let (id, secret) = resolve_client_credentials(&headers, &form).unwrap();
        assert_eq!(id, "client-1");
        assert!(secret.is_none());
    }

    #[test]
    fn missing_client_id_is_rejected() {
        let headers = HeaderMap::new();
        let form = TokenForm {
            grant_type: "authorization_code".to_string(),
            code: Some("c".to_string()),
            redirect_uri: None,
            client_id: None,
            client_secret: None,
            code_verifier: None,
        };
        let err = resolve_client_credentials(&headers, &form).unwrap_err();
        assert_eq!(err.code(), "invalid_client");
    }
}
