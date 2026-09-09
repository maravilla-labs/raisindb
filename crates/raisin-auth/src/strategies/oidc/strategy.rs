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

//! `AuthStrategy` trait implementation for `OidcStrategy`.
//!
//! The trait is the shape every strategy shares. Two of its methods cannot
//! express the OIDC authorization-code flow, and both say so rather than
//! pretending: `get_authorization_url` has nowhere to return the PKCE verifier
//! it must mint, and `handle_callback` has nowhere to receive it. The real
//! entry points are [`OidcStrategy::begin_login`] and
//! [`OidcStrategy::complete_login`] in `flow.rs`, which pass that secret
//! through the sealed login state.

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthProviderConfig;
use std::collections::HashMap;

use crate::strategy::{AuthCredentials, AuthStrategy, AuthenticationResult, StrategyId};

use super::config::{AttributeMappingConfig, OidcConfig};
use super::OidcStrategy;

#[async_trait]
impl AuthStrategy for OidcStrategy {
    fn id(&self) -> &StrategyId {
        &self.strategy_id
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    async fn init(
        &mut self,
        config: &AuthProviderConfig,
        decrypted_secret: Option<&str>,
    ) -> Result<()> {
        let client_id = config
            .client_id
            .as_ref()
            .ok_or_else(|| Error::Validation("OIDC provider requires client_id".to_string()))?;

        let client_secret = decrypted_secret
            .ok_or_else(|| Error::Validation("OIDC provider requires client_secret".to_string()))?;

        let scopes = if config.scopes.is_empty() {
            vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ]
        } else {
            config.scopes.clone()
        };

        let attribute_mapping = AttributeMappingConfig {
            email_claim: non_empty(&config.attribute_mapping.email, "email"),
            name_claim: non_empty(&config.attribute_mapping.name, "name"),
            picture_claim: non_empty(&config.attribute_mapping.picture, "picture"),
            email_verified_claim: non_empty(
                &config.attribute_mapping.email_verified,
                "email_verified",
            ),
        };

        let endpoints = resolve_endpoints(config).await?;

        let oidc_config = OidcConfig {
            client_id: client_id.clone(),
            client_secret: client_secret.to_string(),
            issuer: endpoints.issuer,
            authorization_endpoint: endpoints.authorization_endpoint,
            token_endpoint: endpoints.token_endpoint,
            userinfo_endpoint: endpoints.userinfo_endpoint,
            jwks_uri: config.jwks_url.clone().or(endpoints.jwks_uri),
            // Empty is permitted here and refused at `begin_login`, so a
            // provider can be listed and inspected before its callback URL is
            // registered, without the login silently using a wrong one.
            redirect_uri: config.redirect_uri.clone().unwrap_or_default(),
            scopes,
            attribute_mapping,
            groups_claim: config.groups_claim.clone(),
            allowed_email_domains: config.allowed_email_domains.clone(),
        };

        self.config
            .set(oidc_config)
            .map_err(|_| Error::invalid_state("OidcStrategy already initialized"))?;

        Ok(())
    }

    async fn authenticate(
        &self,
        _tenant_id: &str,
        credentials: AuthCredentials,
    ) -> Result<AuthenticationResult> {
        match credentials {
            AuthCredentials::OAuth2Code { .. } => Err(Error::invalid_state(
                "an OIDC code exchange needs the PKCE verifier from the sealed login state; \
                 call complete_login()",
            )),

            AuthCredentials::OAuth2RefreshToken { .. } => Err(Error::invalid_state(
                "refreshing at the provider is not implemented; RaisinDB issues its own session \
                 tokens once, at login",
            )),

            _ => Err(Error::Validation(
                "OIDC strategy requires OAuth2Code or OAuth2RefreshToken credentials".to_string(),
            )),
        }
    }

    async fn get_authorization_url(
        &self,
        _tenant_id: &str,
        _state: &str,
        _redirect_uri: &str,
    ) -> Result<Option<String>> {
        // Deliberately not implemented. The flow mints a PKCE verifier that
        // must reach the callback, and this signature returns only a URL. The
        // previous version logged the verifier at WARN and threw it away,
        // which left the exchange to run with a placeholder.
        Err(Error::invalid_state(
            "use begin_login(), which returns the authorization URL together with the sealed \
             state carrying the PKCE verifier",
        ))
    }

    async fn handle_callback(
        &self,
        _tenant_id: &str,
        params: HashMap<String, String>,
    ) -> Result<AuthenticationResult> {
        // Validated in the order a caller would hit them, so a missing `code`
        // is reported as such rather than as a missing verifier.
        if !params.contains_key("code") {
            return Err(Error::Validation(
                "Missing 'code' parameter in callback".to_string(),
            ));
        }
        if !params.contains_key("state") {
            return Err(Error::Validation(
                "Missing 'state' parameter in callback".to_string(),
            ));
        }
        Err(Error::invalid_state(
            "use complete_login(), which opens the sealed state to recover the PKCE verifier \
             and the nonce this callback must be checked against",
        ))
    }

    fn supports(&self, credentials: &AuthCredentials) -> bool {
        matches!(
            credentials,
            AuthCredentials::OAuth2Code { .. } | AuthCredentials::OAuth2RefreshToken { .. }
        )
    }
}

/// Fall back to a default when a configured claim name is blank.
fn non_empty(configured: &str, fallback: &str) -> String {
    if configured.is_empty() {
        fallback.to_string()
    } else {
        configured.to_string()
    }
}

/// The endpoints a login needs, however they were obtained.
pub(super) struct ResolvedEndpoints {
    pub(super) issuer: String,
    pub(super) authorization_endpoint: String,
    pub(super) token_endpoint: String,
    pub(super) userinfo_endpoint: Option<String>,
    pub(super) jwks_uri: Option<String>,
}

/// Resolve OIDC endpoints from discovery, falling back to manual config.
///
/// Discovery failing is not fatal, because a provider behind a slow network or
/// a temporary outage should not take down a login path that has every endpoint
/// written down. It is logged at WARN: a deployment that meant to use discovery
/// and quietly fell back would otherwise never learn that its `jwks_uri` is
/// whatever was configured months ago.
async fn resolve_endpoints(config: &AuthProviderConfig) -> Result<ResolvedEndpoints> {
    if let Some(issuer_url) = &config.issuer_url {
        match OidcStrategy::discover_endpoints(issuer_url).await {
            Ok(doc) => {
                return Ok(ResolvedEndpoints {
                    issuer: doc.issuer,
                    authorization_endpoint: doc.authorization_endpoint,
                    token_endpoint: doc.token_endpoint,
                    userinfo_endpoint: doc.userinfo_endpoint,
                    jwks_uri: doc.jwks_uri,
                })
            }
            Err(e) => {
                tracing::warn!(
                    issuer = %issuer_url,
                    error = %e,
                    "OIDC discovery failed; falling back to the manually configured endpoints"
                );
            }
        }
    }
    extract_manual_endpoints(config)
}

/// Extract endpoints from manual config fields.
fn extract_manual_endpoints(config: &AuthProviderConfig) -> Result<ResolvedEndpoints> {
    let authorization_endpoint = config.authorization_url.clone().ok_or_else(|| {
        Error::Validation("OIDC provider requires issuer_url or authorization_url".to_string())
    })?;

    let token_endpoint = config
        .token_url
        .clone()
        .ok_or_else(|| Error::Validation("OIDC provider requires token_url".to_string()))?;

    Ok(ResolvedEndpoints {
        // With no discovery document there is no canonical issuer to compare
        // against, so the configured issuer_url is used verbatim. A provider
        // that spells its issuer differently must be configured with the
        // spelling it puts in the token.
        issuer: config.issuer_url.clone().unwrap_or_default(),
        authorization_endpoint,
        token_endpoint,
        userinfo_endpoint: config.userinfo_url.clone(),
        jwks_uri: config.jwks_url.clone(),
    })
}
