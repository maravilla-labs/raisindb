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

//! Authentication strategy trait and types.
//!
//! Implements a passport.js-style pluggable authentication pattern.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::auth::AuthProviderConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Unique identifier for an authentication strategy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StrategyId(pub String);

impl StrategyId {
    /// Local username/password authentication
    pub const LOCAL: &'static str = "local";
    /// Magic link (passwordless email)
    pub const MAGIC_LINK: &'static str = "magic_link";
    /// One-time token (API access, invites)
    pub const ONE_TIME_TOKEN: &'static str = "one_time_token";
    /// Generic OIDC provider prefix
    pub const OIDC_PREFIX: &'static str = "oidc:";
    /// Generic SAML provider prefix
    pub const SAML_PREFIX: &'static str = "saml:";

    /// Create a new strategy ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Create an OIDC strategy ID
    pub fn oidc(provider: &str) -> Self {
        Self(format!("oidc:{}", provider))
    }

    /// Create a SAML strategy ID
    pub fn saml(provider: &str) -> Self {
        Self(format!("saml:{}", provider))
    }

    /// Check if this is an OIDC strategy
    pub fn is_oidc(&self) -> bool {
        self.0.starts_with(Self::OIDC_PREFIX)
    }

    /// Check if this is a SAML strategy
    pub fn is_saml(&self) -> bool {
        self.0.starts_with(Self::SAML_PREFIX)
    }

    /// Check if this is a local strategy
    pub fn is_local(&self) -> bool {
        self.0 == Self::LOCAL
    }

    /// Get the provider name for OIDC/SAML strategies
    pub fn provider_name(&self) -> Option<&str> {
        if self.is_oidc() {
            Some(&self.0[Self::OIDC_PREFIX.len()..])
        } else if self.is_saml() {
            Some(&self.0[Self::SAML_PREFIX.len()..])
        } else {
            None
        }
    }
}

impl fmt::Display for StrategyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for StrategyId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for StrategyId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for StrategyId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Result of successful authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationResult {
    /// Unique identifier for the authenticated identity
    /// (may be empty if this is a new user - will be assigned later)
    pub identity_id: String,

    /// Email from the provider
    pub email: Option<String>,

    /// Display name from the provider
    pub display_name: Option<String>,

    /// Profile picture URL from the provider
    pub avatar_url: Option<String>,

    /// Provider-specific claims/attributes
    pub provider_claims: HashMap<String, serde_json::Value>,

    /// External provider's user ID (for linking)
    pub external_id: Option<String>,

    /// The strategy that authenticated this user
    pub strategy_id: StrategyId,

    /// Groups/roles from the identity provider (for OIDC/SAML)
    pub provider_groups: Vec<String>,

    /// Whether email has been verified by the provider
    pub email_verified: bool,

    /// Suggested roles to assign (from provider mapping)
    pub suggested_roles: Vec<String>,
}

impl AuthenticationResult {
    /// Create a new authentication result
    pub fn new(strategy_id: StrategyId) -> Self {
        Self {
            identity_id: String::new(),
            email: None,
            display_name: None,
            avatar_url: None,
            provider_claims: HashMap::new(),
            external_id: None,
            strategy_id,
            provider_groups: Vec::new(),
            email_verified: false,
            suggested_roles: Vec::new(),
        }
    }

    /// Set the email
    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// Set the display name
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set email verified status
    pub fn with_email_verified(mut self, verified: bool) -> Self {
        self.email_verified = verified;
        self
    }

    /// Set external ID
    pub fn with_external_id(mut self, id: impl Into<String>) -> Self {
        self.external_id = Some(id.into());
        self
    }
}

/// Credentials for authentication.
///
/// Each variant corresponds to a different authentication method.
#[derive(Debug, Clone)]
pub enum AuthCredentials {
    /// Username/password authentication
    UsernamePassword {
        /// Username or email
        username: String,
        /// Plain text password
        password: String,
    },

    /// Magic link token
    MagicLinkToken {
        /// The token from the magic link
        token: String,
    },

    /// One-time token
    OneTimeToken {
        /// The token value
        token: String,
    },

    /// OAuth2/OIDC authorization code
    OAuth2Code {
        /// Authorization code
        code: String,
        /// CSRF state
        state: String,
        /// Redirect URI used
        redirect_uri: String,
    },

    /// OAuth2/OIDC refresh token
    OAuth2RefreshToken {
        /// Refresh token
        refresh_token: String,
    },

    /// API Key authentication
    ApiKey {
        /// API key value
        key: String,
    },
}

/// The core authentication strategy trait.
///
/// Implements a passport.js-style pattern where each authentication
/// method (local, OIDC, SAML, etc.) is a pluggable strategy.
///
/// # Lifecycle
///
/// 1. Strategy is created and registered with `AuthStrategyRegistry`
/// 2. `init()` is called once at startup with configuration and decrypted secrets
/// 3. `authenticate()` is called for each authentication attempt
/// 4. For redirect-based flows (OIDC/SAML):
///    - `get_authorization_url()` returns the redirect URL
///    - User authenticates with the provider
///    - `handle_callback()` processes the callback
/// 5. `handle_logout()` is called when user logs out (for back-channel logout)
#[async_trait]
pub trait AuthStrategy: Send + Sync {
    /// Get the strategy identifier
    fn id(&self) -> &StrategyId;

    /// Get human-readable name for UI
    fn name(&self) -> &str;

    /// Initialize the strategy with configuration and decrypted secrets.
    ///
    /// Called once at startup. Secrets are decrypted before this call
    /// and should be stored in memory for the lifetime of the strategy.
    async fn init(
        &mut self,
        config: &AuthProviderConfig,
        decrypted_secret: Option<&str>,
    ) -> Result<()>;

    /// Authenticate with the given credentials.
    ///
    /// Returns an `AuthenticationResult` on success, which contains
    /// the user's identity information from the provider.
    async fn authenticate(
        &self,
        tenant_id: &str,
        credentials: AuthCredentials,
    ) -> Result<AuthenticationResult>;

    /// Get the authorization URL for redirect-based flows (OAuth2/OIDC/SAML).
    ///
    /// Returns `None` for strategies that don't use redirects (e.g., local auth).
    async fn get_authorization_url(
        &self,
        tenant_id: &str,
        state: &str,
        redirect_uri: &str,
    ) -> Result<Option<String>> {
        let _ = (tenant_id, state, redirect_uri);
        Ok(None) // Default: not a redirect-based strategy
    }

    /// Handle callback for redirect-based flows.
    ///
    /// Called after the user returns from the identity provider.
    async fn handle_callback(
        &self,
        tenant_id: &str,
        params: HashMap<String, String>,
    ) -> Result<AuthenticationResult> {
        let _ = (tenant_id, params);
        Err(raisin_error::Error::invalid_state(
            "This strategy does not support callbacks",
        ))
    }

    /// Handle logout (e.g., OIDC back-channel logout, token revocation).
    ///
    /// Called when a user logs out to notify the provider if needed.
    async fn handle_logout(&self, identity_id: &str) -> Result<()> {
        let _ = identity_id;
        Ok(()) // Default: no-op for strategies without back-channel logout
    }

    /// Validate a token issued by this strategy (if applicable).
    async fn validate_token(&self, token: &str) -> Result<Option<AuthenticationResult>> {
        let _ = token;
        Ok(None) // Default: does not issue tokens
    }

    /// Check if this strategy supports the given credential type.
    fn supports(&self, credentials: &AuthCredentials) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_id() {
        let local = StrategyId::new("local");
        assert!(local.is_local());
        assert!(!local.is_oidc());

        let google = StrategyId::oidc("google");
        assert!(google.is_oidc());
        assert_eq!(google.provider_name(), Some("google"));

        let azure = StrategyId::saml("azure");
        assert!(azure.is_saml());
        assert_eq!(azure.provider_name(), Some("azure"));
    }

    #[test]
    fn test_authentication_result_builder() {
        let result = AuthenticationResult::new(StrategyId::new("local"))
            .with_email("user@example.com")
            .with_display_name("Test User")
            .with_email_verified(true);

        assert_eq!(result.email, Some("user@example.com".to_string()));
        assert_eq!(result.display_name, Some("Test User".to_string()));
        assert!(result.email_verified);
    }
}
