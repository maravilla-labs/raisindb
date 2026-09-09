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

//! Internal configuration types for the OIDC strategy.

/// OpenID Connect provider configuration (discovered or manual).
///
/// Populated during `init()` either via OIDC discovery or from manually
/// configured endpoints.
#[derive(Debug, Clone)]
pub(super) struct OidcConfig {
    /// Client ID
    pub(super) client_id: String,

    /// Client secret (decrypted)
    pub(super) client_secret: String,

    /// The issuer identifier the `id_token` must claim.
    ///
    /// Taken from the discovery document rather than from the configured URL,
    /// because a provider is entitled to a different canonical spelling (Google
    /// is configured as `https://accounts.google.com` and issues tokens as
    /// `accounts.google.com`). Comparing against the configured string would
    /// reject every Google login.
    pub(super) issuer: String,

    /// Authorization endpoint
    pub(super) authorization_endpoint: String,

    /// Token endpoint
    pub(super) token_endpoint: String,

    /// User info endpoint, if the provider publishes one.
    ///
    /// Optional because the `id_token` already carries the claims we map. The
    /// userinfo call is a supplement for providers that keep `profile` claims
    /// out of the token, not a requirement.
    pub(super) userinfo_endpoint: Option<String>,

    /// JWKS endpoint whose keys verify the `id_token` signature.
    pub(super) jwks_uri: Option<String>,

    /// The exact redirect URI registered with the provider. Sent unchanged at
    /// both the authorization and the token endpoint.
    pub(super) redirect_uri: String,

    /// Requested scopes
    pub(super) scopes: Vec<String>,

    /// Attribute mapping configuration
    pub(super) attribute_mapping: AttributeMappingConfig,

    /// Groups claim name (e.g., "groups", "roles")
    pub(super) groups_claim: Option<String>,

    /// Email domains permitted to sign in. Empty means any.
    pub(super) allowed_email_domains: Vec<String>,
}

/// Attribute mapping configuration extracted from AuthProviderConfig
#[derive(Debug, Clone)]
pub(super) struct AttributeMappingConfig {
    pub(super) email_claim: String,
    pub(super) name_claim: String,
    pub(super) picture_claim: String,
    pub(super) email_verified_claim: String,
}

impl Default for AttributeMappingConfig {
    fn default() -> Self {
        Self {
            email_claim: "email".to_string(),
            name_claim: "name".to_string(),
            picture_claim: "picture".to_string(),
            email_verified_claim: "email_verified".to_string(),
        }
    }
}

/// Token response from the OAuth2 token endpoint.
#[derive(Debug, serde::Deserialize)]
pub(super) struct TokenResponse {
    #[serde(default)]
    pub(super) access_token: Option<String>,
    #[serde(default)]
    pub(super) id_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) expires_in: Option<u64>,
}

/// OpenID Connect discovery document.
///
/// Only the members this client uses are modelled. `userinfo_endpoint` and
/// `jwks_uri` are optional so a minimal but valid provider document still
/// parses; a missing `jwks_uri` is caught later, when a signature actually
/// needs verifying, with an error naming the provider.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct DiscoveryDocument {
    pub(super) issuer: String,
    pub(super) authorization_endpoint: String,
    pub(super) token_endpoint: String,
    #[serde(default)]
    pub(super) userinfo_endpoint: Option<String>,
    #[serde(default)]
    pub(super) jwks_uri: Option<String>,
}

/// A JSON Web Key Set as published at `jwks_uri`.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct JwkSet {
    #[serde(default)]
    pub(super) keys: Vec<Jwk>,
}

/// One RSA verification key from a JWKS.
///
/// `kty` is retained so a set mixing key types (an EC key alongside the RSA
/// ones, which Azure AD has published) is filtered rather than misread: an EC
/// key has no `n`/`e` and would otherwise be a confusing decode failure.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct Jwk {
    #[serde(default)]
    pub(super) kty: Option<String>,
    #[serde(default)]
    pub(super) kid: Option<String>,
    #[serde(default)]
    pub(super) alg: Option<String>,
    /// RSA modulus, base64url.
    #[serde(default)]
    pub(super) n: Option<String>,
    /// RSA exponent, base64url.
    #[serde(default)]
    pub(super) e: Option<String>,
}

impl Jwk {
    /// Whether this key can verify an RS256 signature.
    pub(super) fn is_rsa(&self) -> bool {
        self.kty.as_deref() == Some("RSA") && self.n.is_some() && self.e.is_some()
    }
}
