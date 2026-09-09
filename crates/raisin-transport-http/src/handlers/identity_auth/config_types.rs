// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Types for tenant authentication configuration.

use raisin_models::auth::AttributeMapping;
use serde::{Deserialize, Serialize};

/// Tenant authentication configuration response
#[derive(Debug, Serialize)]
pub struct TenantAuthConfigResponse {
    pub tenant_id: String,
    pub local_auth: LocalAuthConfig,
    pub magic_link: MagicLinkConfig,
    /// External OpenID Connect providers. Secrets are never included; see
    /// [`OidcProviderView::has_client_secret`].
    pub oidc_providers: Vec<OidcProviderView>,
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
            oidc_providers: super::config_oidc::oidc_provider_views(config),
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

/// An OpenID Connect provider as the config endpoints report it.
///
/// The client secret is sealed at rest and never echoed; `has_client_secret`
/// tells an admin UI whether one is stored without revealing it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OidcProviderView {
    /// Slug used in the login URL: `/auth/oidc/{provider_id}`.
    pub provider_id: String,
    pub display_name: String,
    pub icon: String,
    pub enabled: bool,
    pub priority: u32,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub has_client_secret: bool,
    /// The callback URL registered with the provider, byte for byte.
    pub redirect_uri: Option<String>,
    pub scopes: Vec<String>,
    pub attribute_mapping: AttributeMapping,
    pub groups_claim: Option<String>,
    pub allowed_email_domains: Vec<String>,
    /// Manual endpoints, only needed for a provider without discovery.
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub userinfo_url: Option<String>,
    pub jwks_url: Option<String>,
    /// Where a browser starts a login with this provider (server-relative).
    pub authorize_url: String,
}

/// One OpenID Connect provider in a `PUT` request.
///
/// `oidc_providers` on the request is a complete list: a provider absent from
/// it is removed. Within an entry, an omitted `client_secret` keeps the secret
/// already stored for that `provider_id`, so an admin can edit scopes without
/// re-entering the secret.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OidcProviderInput {
    pub provider_id: String,
    pub display_name: Option<String>,
    pub icon: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<u32>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    /// Plaintext on the way in only. Sealed before it is stored.
    pub client_secret: Option<String>,
    pub redirect_uri: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub attribute_mapping: Option<AttributeMapping>,
    pub groups_claim: Option<String>,
    pub allowed_email_domains: Option<Vec<String>>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub userinfo_url: Option<String>,
    pub jwks_url: Option<String>,
}

/// Request to update tenant authentication configuration
#[derive(Debug, Deserialize)]
pub struct UpdateTenantAuthConfigRequest {
    pub local_auth: Option<LocalAuthConfig>,
    pub magic_link: Option<MagicLinkConfig>,
    /// Complete replacement of the OIDC provider list when present.
    pub oidc_providers: Option<Vec<OidcProviderInput>>,
    pub password_policy: Option<PasswordPolicyConfig>,
    pub session_settings: Option<SessionSettingsConfig>,
    pub access_settings: Option<AccessSettingsConfig>,
    pub anonymous_enabled: Option<bool>,
    pub cors_allowed_origins: Option<Vec<String>>,
}
