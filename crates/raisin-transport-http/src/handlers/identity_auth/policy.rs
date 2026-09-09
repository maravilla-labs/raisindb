// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The tenant's effective authentication policy: password rules, login
//! lockout and token lifetimes.
//!
//! These come from the tenant's stored [`TenantAuthConfig`] (edited through
//! `PUT /api/tenants/{tenant}/auth/config`). A tenant that has never stored a
//! configuration gets [`EffectiveAuthPolicy::default`], which reproduces the
//! values the identity endpoints used before the config was consulted at all:
//! an 8-character password minimum, lockout after 5 failures for 15 minutes,
//! and one-hour / thirty-day tokens. So existing tenants behave exactly as
//! before until an admin writes a configuration.

use axum::http::StatusCode;

use crate::error::ApiError;
use crate::state::AppState;

use raisin_models::auth::{PasswordPolicy, TenantAuthConfig};
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::TokenLifetimes;

/// Password minimum applied when no tenant configuration is stored.
pub const LEGACY_MIN_PASSWORD_LENGTH: usize = 8;
/// Failed attempts before lockout when no tenant configuration is stored.
pub const LEGACY_LOCKOUT_THRESHOLD: u32 = 5;
/// Lockout duration in minutes when no tenant configuration is stored.
pub const LEGACY_LOCKOUT_MINUTES: u64 = 15;

/// What the identity endpoints enforce for one tenant.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveAuthPolicy {
    /// `None` means the legacy fixed rule (minimum 8 characters).
    pub password_policy: Option<PasswordPolicy>,
    pub lockout_threshold: u32,
    pub lockout_duration_minutes: u64,
    /// Access token lifetime in seconds.
    pub access_token_seconds: u64,
    /// Refresh token lifetime in seconds.
    pub refresh_token_seconds: u64,
}

impl Default for EffectiveAuthPolicy {
    fn default() -> Self {
        Self {
            password_policy: None,
            lockout_threshold: LEGACY_LOCKOUT_THRESHOLD,
            lockout_duration_minutes: LEGACY_LOCKOUT_MINUTES,
            access_token_seconds: 3600,
            refresh_token_seconds: 30 * 24 * 3600,
        }
    }
}

impl EffectiveAuthPolicy {
    /// Derive the policy from a stored tenant configuration.
    pub fn from_config(config: &TenantAuthConfig) -> Self {
        let threshold = config.rate_limiting.lockout_threshold;
        let minutes = u64::from(config.rate_limiting.lockout_duration_minutes);
        let access = config.session_settings.access_token_duration_seconds;
        let refresh = config.session_settings.refresh_token_duration_seconds;
        let defaults = Self::default();
        Self {
            password_policy: Some(config.password_policy.clone()),
            // A zero threshold would lock every account on its first failure,
            // and a zero duration is not a lockout: both mean "unset".
            lockout_threshold: if threshold == 0 {
                defaults.lockout_threshold
            } else {
                threshold
            },
            lockout_duration_minutes: if minutes == 0 {
                defaults.lockout_duration_minutes
            } else {
                minutes
            },
            access_token_seconds: if access == 0 {
                defaults.access_token_seconds
            } else {
                access
            },
            refresh_token_seconds: if refresh == 0 {
                defaults.refresh_token_seconds
            } else {
                refresh
            },
        }
    }

    /// Validate a password against this policy.
    ///
    /// Errors carry code `WEAK_PASSWORD` and list every unmet rule.
    pub fn validate_password(&self, password: &str) -> Result<(), ApiError> {
        match &self.password_policy {
            None => {
                if password.len() < LEGACY_MIN_PASSWORD_LENGTH {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "WEAK_PASSWORD",
                        format!(
                            "Password must be at least {} characters long",
                            LEGACY_MIN_PASSWORD_LENGTH
                        ),
                    ));
                }
                Ok(())
            }
            Some(policy) => policy.validate(password).map_err(|errors| {
                ApiError::new(StatusCode::BAD_REQUEST, "WEAK_PASSWORD", errors.join("; "))
            }),
        }
    }

    #[cfg(feature = "storage-rocksdb")]
    pub fn token_lifetimes(&self) -> TokenLifetimes {
        TokenLifetimes {
            access_seconds: self.access_token_seconds,
            refresh_seconds: self.refresh_token_seconds,
        }
        .sanitized()
    }
}

/// Load the effective policy for a tenant.
///
/// A missing configuration yields the defaults. A storage error is logged and
/// also yields the defaults, so a broken config store degrades to the
/// long-standing behaviour rather than locking everyone out.
pub async fn load_auth_policy(state: &AppState, tenant_id: &str) -> EffectiveAuthPolicy {
    match state
        .storage()
        .tenant_auth_config_repository()
        .get_config(tenant_id)
        .await
    {
        Ok(Some(config)) => EffectiveAuthPolicy::from_config(&config),
        Ok(None) => EffectiveAuthPolicy::default(),
        Err(e) => {
            tracing::warn!(
                tenant_id = %tenant_id,
                error = %e,
                "Failed to load tenant auth config; using default auth policy"
            );
            EffectiveAuthPolicy::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::auth::{PasswordPolicy, RateLimitSettings, SessionSettings};

    #[test]
    fn default_policy_matches_the_previous_hardcoded_behaviour() {
        let p = EffectiveAuthPolicy::default();
        assert_eq!(p.lockout_threshold, 5);
        assert_eq!(p.lockout_duration_minutes, 15);
        assert_eq!(p.access_token_seconds, 3600);
        assert_eq!(p.refresh_token_seconds, 30 * 24 * 3600);
        // Legacy rule: 8 characters, nothing else.
        assert!(p.validate_password("short").is_err());
        assert!(p.validate_password("alllowercaseletters").is_ok());
    }

    #[test]
    fn stored_config_is_enforced() {
        let mut config = TenantAuthConfig::new("t".to_string());
        config.password_policy = PasswordPolicy {
            min_length: 12,
            require_uppercase: true,
            require_digit: true,
            ..PasswordPolicy::default()
        };
        config.rate_limiting = RateLimitSettings {
            lockout_threshold: 3,
            lockout_duration_minutes: 45,
            ..RateLimitSettings::default()
        };
        config.session_settings = SessionSettings {
            access_token_duration_seconds: 900,
            refresh_token_duration_seconds: 86_400,
            ..SessionSettings::default()
        };

        let p = EffectiveAuthPolicy::from_config(&config);
        assert_eq!(p.lockout_threshold, 3);
        assert_eq!(p.lockout_duration_minutes, 45);
        assert_eq!(p.access_token_seconds, 900);
        assert_eq!(p.refresh_token_seconds, 86_400);

        let err = p.validate_password("alllowercaseletters").unwrap_err();
        assert_eq!(err.code, "WEAK_PASSWORD");
        assert!(err.message.contains("uppercase"), "{}", err.message);
        assert!(err.message.contains("digit"), "{}", err.message);
        assert!(p.validate_password("CorrectHorse42Battery").is_ok());
        assert!(p.validate_password("Short1").is_err());
    }

    #[test]
    fn zero_values_in_stored_config_fall_back_to_defaults() {
        let mut config = TenantAuthConfig::new("t".to_string());
        config.rate_limiting.lockout_threshold = 0;
        config.rate_limiting.lockout_duration_minutes = 0;
        config.session_settings.access_token_duration_seconds = 0;
        config.session_settings.refresh_token_duration_seconds = 0;
        let p = EffectiveAuthPolicy::from_config(&config);
        let d = EffectiveAuthPolicy::default();
        assert_eq!(p.lockout_threshold, d.lockout_threshold);
        assert_eq!(p.lockout_duration_minutes, d.lockout_duration_minutes);
        assert_eq!(p.access_token_seconds, d.access_token_seconds);
        assert_eq!(p.refresh_token_seconds, d.refresh_token_seconds);
    }
}
