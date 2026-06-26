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

//! Discovery metadata documents.
//!
//! - [`AuthorizationServerMetadata`] is the RFC 8414 authorization-server
//!   metadata served at `/.well-known/oauth-authorization-server`.
//! - [`ProtectedResourceMetadata`] is the OAuth 2.0 Protected Resource Metadata
//!   (RFC 9728) an MCP client fetches at
//!   `/.well-known/oauth-protected-resource/...` to discover which
//!   authorization server guards a given MCP resource.

use serde::Serialize;

/// RFC 8414 authorization-server metadata.
///
/// Only the fields RaisinDB actually implements are advertised, so a client's
/// capability negotiation never promises an endpoint that does not exist.
#[derive(Debug, Clone, Serialize)]
pub struct AuthorizationServerMetadata {
    /// The issuer identifier (a URL with no query or fragment).
    pub issuer: String,
    /// The authorization endpoint URL.
    pub authorization_endpoint: String,
    /// The token endpoint URL.
    pub token_endpoint: String,
    /// The dynamic client registration endpoint URL (RFC 7591).
    pub registration_endpoint: String,
    /// Response types supported — `code` only (OAuth 2.1).
    pub response_types_supported: Vec<String>,
    /// Grant types supported.
    pub grant_types_supported: Vec<String>,
    /// PKCE challenge methods supported — `S256` only (OAuth 2.1).
    pub code_challenge_methods_supported: Vec<String>,
    /// Token-endpoint client authentication methods supported.
    pub token_endpoint_auth_methods_supported: Vec<String>,
    /// Scopes the server may advertise. Empty when scopes are tenant-defined.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,
}

impl AuthorizationServerMetadata {
    /// Build metadata for an issuer rooted at `issuer` (e.g. `https://host`).
    ///
    /// The endpoint URLs are derived by appending the server's fixed paths to
    /// the issuer, matching the routes the HTTP transport mounts.
    pub fn for_issuer(issuer: impl Into<String>) -> Self {
        let issuer = issuer.into();
        let base = issuer.trim_end_matches('/').to_string();
        Self {
            authorization_endpoint: format!("{base}/authorize"),
            token_endpoint: format!("{base}/token"),
            registration_endpoint: format!("{base}/register"),
            issuer: base,
            response_types_supported: vec!["code".to_string()],
            grant_types_supported: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            code_challenge_methods_supported: vec!["S256".to_string()],
            token_endpoint_auth_methods_supported: vec![
                "none".to_string(),
                "client_secret_post".to_string(),
                "client_secret_basic".to_string(),
            ],
            scopes_supported: Vec::new(),
        }
    }

    /// Advertise a fixed scope catalogue.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes_supported = scopes;
        self
    }
}

/// RFC 9728 protected-resource metadata for a single MCP resource.
///
/// An MCP client that receives a `401` with a
/// `WWW-Authenticate: Bearer resource_metadata="..."` challenge fetches this
/// document to learn which authorization server(s) can issue a token for the
/// resource and which scopes the resource recognizes.
#[derive(Debug, Clone, Serialize)]
pub struct ProtectedResourceMetadata {
    /// The canonical resource identifier (the MCP server URL).
    pub resource: String,
    /// Authorization servers that can issue tokens for this resource.
    pub authorization_servers: Vec<String>,
    /// Scopes this resource understands. Empty when scopes are tenant-defined.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,
    /// How a bearer token may be presented — header only.
    pub bearer_methods_supported: Vec<String>,
}

impl ProtectedResourceMetadata {
    /// Build metadata for `resource`, guarded by the authorization server at
    /// `issuer`.
    pub fn new(resource: impl Into<String>, issuer: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            authorization_servers: vec![issuer.into().trim_end_matches('/').to_string()],
            scopes_supported: Vec::new(),
            bearer_methods_supported: vec!["header".to_string()],
        }
    }

    /// Advertise the scopes this resource recognizes.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes_supported = scopes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuer_endpoints_are_derived() {
        let md = AuthorizationServerMetadata::for_issuer("https://db.example.com/");
        assert_eq!(md.issuer, "https://db.example.com");
        assert_eq!(md.authorization_endpoint, "https://db.example.com/authorize");
        assert_eq!(md.token_endpoint, "https://db.example.com/token");
        assert_eq!(md.registration_endpoint, "https://db.example.com/register");
        assert_eq!(md.code_challenge_methods_supported, vec!["S256"]);
    }

    #[test]
    fn protected_resource_points_at_issuer() {
        let md = ProtectedResourceMetadata::new(
            "https://db.example.com/mcp/repo/main/srv",
            "https://db.example.com",
        );
        assert_eq!(md.authorization_servers, vec!["https://db.example.com"]);
        assert_eq!(md.bearer_methods_supported, vec!["header"]);
    }
}
