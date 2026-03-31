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

//! Tenant-level authentication configuration.

use serde::{Deserialize, Serialize};

use super::policies::{PasswordPolicy, RateLimitSettings, SessionSettings};
use super::provider_config::AuthProviderConfig;
use crate::auth::access::AccessSettings;

fn default_true() -> bool {
    true
}

/// Tenant-level authentication configuration.
///
/// Similar to `TenantAIConfig`, this stores all authentication
/// settings and provider configurations for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TenantAuthConfig {
    /// Tenant ID
    pub tenant_id: String,

    /// Configured authentication providers
    pub providers: Vec<AuthProviderConfig>,

    /// Default access settings for workspaces
    pub access_settings: AccessSettings,

    /// Session settings
    pub session_settings: SessionSettings,

    /// Password policy (for local auth)
    pub password_policy: PasswordPolicy,

    /// Rate limiting settings
    pub rate_limiting: RateLimitSettings,

    /// Whether to enable audit logging for auth events
    #[serde(default = "default_true")]
    pub audit_enabled: bool,

    /// Whether anonymous (unauthenticated) access is enabled globally.
    /// When enabled, unauthenticated requests are treated as the "anonymous" user
    /// with permissions defined by the "anonymous" role.
    #[serde(default)]
    pub anonymous_enabled: bool,

    /// CORS allowed origins for this tenant.
    /// These are used as fallback when repository-level CORS is not configured.
    /// Example: ["http://localhost:5173", "https://app.example.com"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cors_allowed_origins: Vec<String>,
}

impl Default for TenantAuthConfig {
    fn default() -> Self {
        Self {
            tenant_id: String::new(),
            providers: Vec::new(),
            access_settings: AccessSettings::default(),
            session_settings: SessionSettings::default(),
            password_policy: PasswordPolicy::default(),
            rate_limiting: RateLimitSettings::default(),
            audit_enabled: true,
            anonymous_enabled: false,
            cors_allowed_origins: Vec::new(),
        }
    }
}

impl TenantAuthConfig {
    /// Create a new config for a tenant
    pub fn new(tenant_id: String) -> Self {
        Self {
            tenant_id,
            ..Default::default()
        }
    }

    /// Get a provider by strategy ID
    pub fn get_provider(&self, strategy_id: &str) -> Option<&AuthProviderConfig> {
        self.providers.iter().find(|p| p.strategy_id == strategy_id)
    }

    /// Get all enabled providers
    pub fn enabled_providers(&self) -> impl Iterator<Item = &AuthProviderConfig> {
        self.providers.iter().filter(|p| p.enabled)
    }

    /// Check if local authentication is enabled
    pub fn local_auth_enabled(&self) -> bool {
        self.providers
            .iter()
            .any(|p| p.strategy_id == "local" && p.enabled)
    }

    /// Check if magic link authentication is enabled
    pub fn magic_link_enabled(&self) -> bool {
        self.providers
            .iter()
            .any(|p| p.strategy_id == "magic_link" && p.enabled)
    }
}
