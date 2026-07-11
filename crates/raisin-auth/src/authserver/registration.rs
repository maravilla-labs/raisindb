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

//! Dynamic Client Registration (RFC 7591).
//!
//! Validates an incoming registration request, applies OAuth 2.1 defaults
//! suitable for an interactive MCP client (public client, PKCE-only,
//! authorization-code + refresh-token grants), mints a client id (and a secret
//! for confidential clients), and returns both the persisted [`OAuthClient`]
//! and the wire response.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

use super::error::{AuthServerError, AuthServerResult};
use super::model::{
    ClientRegistrationRequest, ClientRegistrationResponse, OAuthClient, TokenEndpointAuthMethod,
};

/// Outcome of a successful registration: the record to persist plus the wire
/// response (the response alone carries the one-time plaintext secret).
#[derive(Debug)]
pub struct RegistrationOutcome {
    /// The client record to store.
    pub client: OAuthClient,
    /// The response body to return to the caller.
    pub response: ClientRegistrationResponse,
}

/// Parse a requested `token_endpoint_auth_method`, defaulting to `none`.
fn parse_auth_method(raw: Option<&str>) -> AuthServerResult<TokenEndpointAuthMethod> {
    match raw {
        None | Some("none") => Ok(TokenEndpointAuthMethod::None),
        Some("client_secret_post") => Ok(TokenEndpointAuthMethod::ClientSecretPost),
        Some("client_secret_basic") => Ok(TokenEndpointAuthMethod::ClientSecretBasic),
        Some(other) => Err(AuthServerError::InvalidClientMetadata(format!(
            "unsupported token_endpoint_auth_method: {other}"
        ))),
    }
}

/// Reject a redirect URI that cannot be used securely.
///
/// OAuth 2.1 requires absolute URIs with no fragment. We additionally require
/// either the `https` scheme or a loopback `http` host (the interactive-client
/// exception), which is exactly what native MCP clients use.
fn validate_redirect_uri(uri: &str) -> AuthServerResult<()> {
    let parsed = url::Url::parse(uri).map_err(|e| {
        AuthServerError::InvalidClientMetadata(format!("redirect_uri '{uri}' is not a URI: {e}"))
    })?;
    if parsed.fragment().is_some() {
        return Err(AuthServerError::InvalidClientMetadata(format!(
            "redirect_uri '{uri}' must not contain a fragment"
        )));
    }
    let is_loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if is_loopback => Ok(()),
        // Custom/native app schemes (e.g. `myapp://callback`) are permitted for
        // public clients that cannot host an https endpoint.
        scheme if !scheme.is_empty() && scheme != "http" => Ok(()),
        _ => Err(AuthServerError::InvalidClientMetadata(format!(
            "redirect_uri '{uri}' must be https or a loopback http address"
        ))),
    }
}

/// Generate a random base64url client id with a recognizable prefix.
fn generate_client_id() -> String {
    use rand::Rng;
    let bytes: [u8; 16] = rand::thread_rng().gen();
    format!("client_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Generate a random client secret and its SHA-256 hex digest.
fn generate_client_secret() -> (String, String) {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    let secret = URL_SAFE_NO_PAD.encode(bytes);
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    (secret, hex_encode(&digest))
}

/// Lowercase hex encoding without pulling in an extra dependency.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// SHA-256 hex digest of a presented client secret, for comparison against the
/// stored hash.
pub fn hash_client_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex_encode(&hasher.finalize())
}

/// Process a dynamic client registration request for `tenant_id`.
///
/// `now` is the current Unix timestamp (passed in for deterministic tests).
pub fn register_client(
    tenant_id: &str,
    req: ClientRegistrationRequest,
    now: i64,
) -> AuthServerResult<RegistrationOutcome> {
    // Redirect URIs are mandatory for the authorization-code grant.
    if req.redirect_uris.is_empty() {
        return Err(AuthServerError::InvalidClientMetadata(
            "at least one redirect_uri is required".to_string(),
        ));
    }
    for uri in &req.redirect_uris {
        validate_redirect_uri(uri)?;
    }

    let auth_method = parse_auth_method(req.token_endpoint_auth_method.as_deref())?;

    // Default to the interactive-client grant set when the request omits it.
    let grant_types = if req.grant_types.is_empty() {
        vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ]
    } else {
        for g in &req.grant_types {
            if !matches!(g.as_str(), "authorization_code" | "refresh_token") {
                return Err(AuthServerError::InvalidClientMetadata(format!(
                    "unsupported grant_type: {g}"
                )));
            }
        }
        req.grant_types.clone()
    };

    let response_types = if req.response_types.is_empty() {
        vec!["code".to_string()]
    } else {
        for r in &req.response_types {
            if r != "code" {
                return Err(AuthServerError::InvalidClientMetadata(format!(
                    "unsupported response_type: {r}"
                )));
            }
        }
        req.response_types.clone()
    };

    let client_id = generate_client_id();

    // Only confidential clients receive a secret.
    let (client_secret, client_secret_hash) = if auth_method.is_confidential() {
        let (secret, hash) = generate_client_secret();
        (Some(secret), Some(hash))
    } else {
        (None, None)
    };

    let client = OAuthClient {
        client_id: client_id.clone(),
        client_secret_hash,
        tenant_id: tenant_id.to_string(),
        client_name: req.client_name.clone(),
        redirect_uris: req.redirect_uris.clone(),
        grant_types: grant_types.clone(),
        response_types: response_types.clone(),
        scope: req.scope.clone(),
        token_endpoint_auth_method: auth_method,
        created_at: now,
    };

    let response = ClientRegistrationResponse {
        client_id,
        client_secret,
        client_id_issued_at: now,
        redirect_uris: req.redirect_uris,
        token_endpoint_auth_method: auth_method,
        grant_types,
        response_types,
        client_name: req.client_name,
        scope: req.scope,
    };

    Ok(RegistrationOutcome { client, response })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request() -> ClientRegistrationRequest {
        ClientRegistrationRequest {
            redirect_uris: vec!["http://localhost:8976/callback".to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn public_client_gets_no_secret_and_default_grants() {
        let out = register_client("tenant-a", base_request(), 1000).unwrap();
        assert!(out.client.client_secret_hash.is_none());
        assert!(out.response.client_secret.is_none());
        assert_eq!(
            out.client.grant_types,
            vec!["authorization_code", "refresh_token"]
        );
        assert_eq!(out.client.response_types, vec!["code"]);
        assert!(out.client.client_id.starts_with("client_"));
    }

    #[test]
    fn confidential_client_gets_secret_and_matching_hash() {
        let mut req = base_request();
        req.token_endpoint_auth_method = Some("client_secret_post".to_string());
        let out = register_client("tenant-a", req, 1000).unwrap();

        let secret = out.response.client_secret.expect("secret returned once");
        let stored = out.client.client_secret_hash.expect("hash stored");
        assert_eq!(hash_client_secret(&secret), stored);
    }

    #[test]
    fn missing_redirect_uri_is_rejected() {
        let err = register_client("tenant-a", ClientRegistrationRequest::default(), 1000)
            .expect_err("must require a redirect_uri");
        assert_eq!(err.code(), "invalid_client_metadata");
    }

    #[test]
    fn non_loopback_http_redirect_is_rejected() {
        let mut req = base_request();
        req.redirect_uris = vec!["http://evil.example.com/cb".to_string()];
        let err =
            register_client("tenant-a", req, 1000).expect_err("non-loopback http must be rejected");
        assert_eq!(err.code(), "invalid_client_metadata");
    }

    #[test]
    fn loopback_http_and_custom_scheme_are_allowed() {
        let mut req = base_request();
        req.redirect_uris = vec![
            "http://127.0.0.1:9000/cb".to_string(),
            "myapp://callback".to_string(),
        ];
        register_client("tenant-a", req, 1000).expect("loopback + custom scheme allowed");
    }
}
