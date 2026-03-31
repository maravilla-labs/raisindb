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
/// This is populated during `init()` either via OIDC discovery or from
/// manually configured endpoints.
#[derive(Debug, Clone)]
pub(super) struct OidcConfig {
    /// Client ID
    pub(super) client_id: String,

    /// Client secret (decrypted)
    #[allow(dead_code)]
    pub(super) client_secret: String,

    /// Authorization endpoint
    pub(super) authorization_endpoint: String,

    /// Token endpoint
    #[allow(dead_code)]
    pub(super) token_endpoint: String,

    /// User info endpoint
    #[allow(dead_code)]
    pub(super) userinfo_endpoint: String,

    /// Requested scopes
    pub(super) scopes: Vec<String>,

    /// Attribute mapping configuration
    pub(super) attribute_mapping: AttributeMappingConfig,

    /// Groups claim name (e.g., "groups", "roles")
    pub(super) groups_claim: Option<String>,
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
    pub(super) access_token: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) id_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) expires_in: Option<u64>,
}

/// OpenID Connect discovery document.
#[derive(Debug, serde::Deserialize)]
pub(super) struct DiscoveryDocument {
    pub(super) authorization_endpoint: String,
    pub(super) token_endpoint: String,
    pub(super) userinfo_endpoint: String,
}
