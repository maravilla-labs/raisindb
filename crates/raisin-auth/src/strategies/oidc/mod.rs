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

//! OpenID Connect (OIDC) authentication strategy.
//!
//! This strategy implements OAuth2/OpenID Connect authentication with support
//! for multiple providers (Google, Okta, Azure AD, Keycloak, etc.).
//!
//! # Features
//!
//! - OAuth2 authorization code flow with PKCE support
//! - Automatic OIDC discovery (via `.well-known/openid-configuration`)
//! - Manual endpoint configuration fallback
//! - Refresh token support
//! - Configurable attribute mapping for user profile
//! - Group/role mapping from provider claims
//!
//! # Module Structure
//!
//! - `config`: OIDC configuration types
//! - `discovery`: OIDC endpoint discovery, JWKS retrieval, token exchange
//! - `flow`: the authorization-code flow, `begin_login` and `complete_login`
//! - `idtoken`: `id_token` signature and claim verification
//! - `login_state`: the sealed `state` parameter carrying the PKCE verifier
//! - `mapping`: User info claim mapping
//! - `pkce`: PKCE code verifier/challenge generation
//! - `strategy`: `AuthStrategy` trait implementation

mod config;
mod discovery;
mod flow;
mod idtoken;
mod login_state;
mod mapping;
mod pkce;
mod strategy;

#[cfg(test)]
mod tests;

use raisin_error::{Error, Result};
use std::sync::OnceLock;

use crate::strategy::StrategyId;

use config::OidcConfig;

pub use flow::LoginRedirect;
pub use login_state::{OidcLoginState, LOGIN_STATE_TTL_SECONDS};

/// OpenID Connect authentication strategy.
///
/// Supports OAuth2/OIDC authentication with multiple providers.
/// Configuration is set once via `init()` and stored internally.
#[derive(Debug)]
pub struct OidcStrategy {
    /// Strategy identifier (e.g., "oidc:google")
    strategy_id: StrategyId,

    /// Display name for UI
    display_name: String,

    /// OIDC configuration (set during init)
    config: OnceLock<OidcConfig>,
}

impl OidcStrategy {
    /// Create a new OIDC strategy.
    ///
    /// # Arguments
    ///
    /// * `provider_name` - Provider identifier (e.g., "google", "okta", "azure")
    /// * `display_name` - Human-readable name for UI
    pub fn new(provider_name: impl Into<String>, display_name: impl Into<String>) -> Self {
        let provider_name = provider_name.into();
        Self {
            strategy_id: StrategyId::oidc(&provider_name),
            display_name: display_name.into(),
            config: OnceLock::new(),
        }
    }

    /// The provider slug, e.g. `google` for the strategy `oidc:google`.
    ///
    /// Falls back to the whole strategy id, which cannot happen for a strategy
    /// built by [`Self::new`] but keeps this total.
    pub fn provider_name(&self) -> &str {
        self.strategy_id
            .provider_name()
            .unwrap_or_else(|| self.strategy_id.as_ref())
    }

    /// Get the OIDC configuration.
    ///
    /// Returns an error if the strategy hasn't been initialized.
    fn get_config(&self) -> Result<&OidcConfig> {
        self.config
            .get()
            .ok_or_else(|| Error::invalid_state("OidcStrategy not initialized - call init() first"))
    }
}
