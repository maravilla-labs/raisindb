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

//! Authentication provider configuration and attribute mapping.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for an authentication provider.
///
/// Secrets are encrypted at rest using AES-256-GCM (like AI provider keys)
/// and decrypted once at strategy initialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthProviderConfig {
    /// Unique provider identifier
    pub provider_id: String,

    /// Strategy ID (e.g., "local", "magic_link", "oidc:google")
    pub strategy_id: String,

    /// Display name for UI
    pub display_name: String,

    /// Icon identifier (Lucide icon name)
    pub icon: String,

    /// Whether this provider is enabled
    pub enabled: bool,

    /// Priority for display order (lower = higher priority)
    #[serde(default)]
    pub priority: u32,

    /// Client ID (for OAuth2/OIDC)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// Client secret encrypted with AES-256-GCM
    /// Decrypted once at strategy initialization, not per-request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret_encrypted: Option<Vec<u8>>,

    /// Issuer URL (for OIDC discovery)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer_url: Option<String>,

    /// Authorization URL (if not using discovery)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,

    /// Token URL (if not using discovery)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,

    /// User info URL (if not using discovery)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub userinfo_url: Option<String>,

    /// Requested scopes
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Attribute mapping from provider claims to identity fields
    #[serde(default)]
    pub attribute_mapping: AttributeMapping,

    /// Claim name containing user groups (for group mapping)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups_claim: Option<String>,

    /// Additional provider-specific configuration
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra_config: HashMap<String, serde_json::Value>,
}

impl AuthProviderConfig {
    /// Create a local auth provider config
    pub fn local() -> Self {
        Self {
            provider_id: "local".to_string(),
            strategy_id: "local".to_string(),
            display_name: "Email & Password".to_string(),
            icon: "key".to_string(),
            enabled: true,
            priority: 100,
            client_id: None,
            client_secret_encrypted: None,
            issuer_url: None,
            authorization_url: None,
            token_url: None,
            userinfo_url: None,
            scopes: Vec::new(),
            attribute_mapping: AttributeMapping::default(),
            groups_claim: None,
            extra_config: HashMap::new(),
        }
    }

    /// Create a magic link provider config
    pub fn magic_link() -> Self {
        Self {
            provider_id: "magic_link".to_string(),
            strategy_id: "magic_link".to_string(),
            display_name: "Magic Link".to_string(),
            icon: "mail".to_string(),
            enabled: true,
            priority: 90,
            client_id: None,
            client_secret_encrypted: None,
            issuer_url: None,
            authorization_url: None,
            token_url: None,
            userinfo_url: None,
            scopes: Vec::new(),
            attribute_mapping: AttributeMapping::default(),
            groups_claim: None,
            extra_config: HashMap::new(),
        }
    }

    /// Create a Google OIDC provider config
    pub fn google(client_id: String) -> Self {
        Self {
            provider_id: "google".to_string(),
            strategy_id: "oidc:google".to_string(),
            display_name: "Sign in with Google".to_string(),
            icon: "chrome".to_string(), // or use a custom google icon
            enabled: true,
            priority: 10,
            client_id: Some(client_id),
            client_secret_encrypted: None,
            issuer_url: Some("https://accounts.google.com".to_string()),
            authorization_url: None,
            token_url: None,
            userinfo_url: None,
            scopes: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ],
            attribute_mapping: AttributeMapping::default(),
            groups_claim: None,
            extra_config: HashMap::new(),
        }
    }

    /// Check if this is an OIDC provider
    pub fn is_oidc(&self) -> bool {
        self.strategy_id.starts_with("oidc:")
    }

    /// Check if this is a SAML provider
    pub fn is_saml(&self) -> bool {
        self.strategy_id.starts_with("saml:")
    }
}

/// Attribute mapping from provider claims to identity fields
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AttributeMapping {
    /// Claim name for email
    #[serde(default = "default_email_claim")]
    pub email: String,

    /// Claim name for display name
    #[serde(default = "default_name_claim")]
    pub name: String,

    /// Claim name for profile picture
    #[serde(default = "default_picture_claim")]
    pub picture: String,

    /// Claim name for email verified status
    #[serde(default = "default_email_verified_claim")]
    pub email_verified: String,

    /// Claim name for first name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,

    /// Claim name for last name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
}

fn default_email_claim() -> String {
    "email".to_string()
}

fn default_name_claim() -> String {
    "name".to_string()
}

fn default_picture_claim() -> String {
    "picture".to_string()
}

fn default_email_verified_claim() -> String {
    "email_verified".to_string()
}
