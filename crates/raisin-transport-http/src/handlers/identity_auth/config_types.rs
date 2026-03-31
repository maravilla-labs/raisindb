// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Types for tenant authentication configuration.

use serde::{Deserialize, Serialize};

/// Tenant authentication configuration response
#[derive(Debug, Serialize)]
pub struct TenantAuthConfigResponse {
    pub tenant_id: String,
    pub local_auth: LocalAuthConfig,
    pub magic_link: MagicLinkConfig,
    pub password_policy: PasswordPolicyConfig,
    pub session_settings: SessionSettingsConfig,
    pub access_settings: AccessSettingsConfig,
    pub anonymous_enabled: bool,
    pub cors_allowed_origins: Vec<String>,
}

impl TenantAuthConfigResponse {
    pub fn from_config(config: &raisin_models::auth::TenantAuthConfig) -> Self {
        Self {
            tenant_id: config.tenant_id.clone(),
            local_auth: LocalAuthConfig {
                enabled: config.local_auth_enabled(),
            },
            magic_link: MagicLinkConfig {
                enabled: config.magic_link_enabled(),
                token_ttl_minutes: 15, // Default, could be configurable
            },
            password_policy: PasswordPolicyConfig {
                min_length: config.password_policy.min_length,
                require_uppercase: config.password_policy.require_uppercase,
                require_lowercase: config.password_policy.require_lowercase,
                require_numbers: config.password_policy.require_digit,
                require_special: config.password_policy.require_special,
                max_age_days: if config.password_policy.expiry_days > 0 {
                    Some(config.password_policy.expiry_days)
                } else {
                    None
                },
            },
            session_settings: SessionSettingsConfig {
                duration_hours: (config.session_settings.access_token_duration_seconds / 3600)
                    as u32,
                refresh_token_duration_days: (config
                    .session_settings
                    .refresh_token_duration_seconds
                    / (24 * 3600)) as u32,
                max_sessions_per_user: config.session_settings.max_sessions_per_user,
                single_session_mode: config.session_settings.max_sessions_per_user == 1,
            },
            access_settings: AccessSettingsConfig {
                allow_access_requests: config.access_settings.allow_access_requests,
                allow_invitations: config.access_settings.allow_invitations,
                require_approval: config.access_settings.require_approval,
                default_roles: config.access_settings.default_roles.clone(),
            },
            anonymous_enabled: config.anonymous_enabled,
            cors_allowed_origins: config.cors_allowed_origins.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalAuthConfig {
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MagicLinkConfig {
    pub enabled: bool,
    pub token_ttl_minutes: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PasswordPolicyConfig {
    pub min_length: u32,
    pub require_uppercase: bool,
    pub require_lowercase: bool,
    pub require_numbers: bool,
    pub require_special: bool,
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSettingsConfig {
    pub duration_hours: u32,
    pub refresh_token_duration_days: u32,
    pub max_sessions_per_user: u32,
    pub single_session_mode: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessSettingsConfig {
    pub allow_access_requests: bool,
    pub allow_invitations: bool,
    pub require_approval: bool,
    pub default_roles: Vec<String>,
}

/// Request to update tenant authentication configuration
#[derive(Debug, Deserialize)]
pub struct UpdateTenantAuthConfigRequest {
    pub local_auth: Option<LocalAuthConfig>,
    pub magic_link: Option<MagicLinkConfig>,
    pub password_policy: Option<PasswordPolicyConfig>,
    pub session_settings: Option<SessionSettingsConfig>,
    pub access_settings: Option<AccessSettingsConfig>,
    pub anonymous_enabled: Option<bool>,
    pub cors_allowed_origins: Option<Vec<String>>,
}
