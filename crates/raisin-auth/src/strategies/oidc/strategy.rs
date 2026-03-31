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
        // Validate required fields
        let client_id = config
            .client_id
            .as_ref()
            .ok_or_else(|| Error::Validation("OIDC provider requires client_id".to_string()))?;

        let client_secret = decrypted_secret
            .ok_or_else(|| Error::Validation("OIDC provider requires client_secret".to_string()))?;

        // Extract scopes with defaults
        let scopes = if config.scopes.is_empty() {
            vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ]
        } else {
            config.scopes.clone()
        };

        // Extract attribute mapping with fallback to defaults if empty
        let attribute_mapping = AttributeMappingConfig {
            email_claim: if config.attribute_mapping.email.is_empty() {
                "email".to_string()
            } else {
                config.attribute_mapping.email.clone()
            },
            name_claim: if config.attribute_mapping.name.is_empty() {
                "name".to_string()
            } else {
                config.attribute_mapping.name.clone()
            },
            picture_claim: if config.attribute_mapping.picture.is_empty() {
                "picture".to_string()
            } else {
                config.attribute_mapping.picture.clone()
            },
            email_verified_claim: if config.attribute_mapping.email_verified.is_empty() {
                "email_verified".to_string()
            } else {
                config.attribute_mapping.email_verified.clone()
            },
        };

        // Determine endpoints: use discovery if issuer_url is provided,
        // otherwise require manual config
        let (authorization_endpoint, token_endpoint, userinfo_endpoint) =
            resolve_endpoints(config).await?;

        // Store configuration
        let oidc_config = OidcConfig {
            client_id: client_id.clone(),
            client_secret: client_secret.to_string(),
            authorization_endpoint,
            token_endpoint,
            userinfo_endpoint,
            scopes,
            attribute_mapping,
            groups_claim: config.groups_claim.clone(),
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
            AuthCredentials::OAuth2Code {
                code,
                redirect_uri,
                state: _,
            } => {
                let _ = (code, redirect_uri);
                Err(Error::invalid_state(
                    "OAuth2 code authentication requires code_verifier from session. \
                     Use handle_callback() instead, which manages the full flow.",
                ))
            }

            AuthCredentials::OAuth2RefreshToken { refresh_token } => {
                let _ = refresh_token;
                Err(Error::invalid_state(
                    "Refresh token authentication is not yet fully implemented. \
                     Enable the 'oidc' feature for real HTTP requests.",
                ))
            }

            _ => Err(Error::Validation(
                "OIDC strategy requires OAuth2Code or OAuth2RefreshToken credentials".to_string(),
            )),
        }
    }

    async fn get_authorization_url(
        &self,
        _tenant_id: &str,
        state: &str,
        redirect_uri: &str,
    ) -> Result<Option<String>> {
        let code_verifier = Self::generate_code_verifier();
        let code_challenge = Self::generate_code_challenge(&code_verifier);

        tracing::warn!(
            "PKCE code_verifier generated but not stored: {}. \
             Store this in session storage keyed by state parameter: {}",
            code_verifier,
            state
        );

        let url = self.build_authorization_url(redirect_uri, state, &code_challenge, None)?;
        Ok(Some(url))
    }

    async fn handle_callback(
        &self,
        _tenant_id: &str,
        params: HashMap<String, String>,
    ) -> Result<AuthenticationResult> {
        let code = params
            .get("code")
            .ok_or_else(|| Error::Validation("Missing 'code' parameter in callback".to_string()))?;

        let state = params.get("state").ok_or_else(|| {
            Error::Validation("Missing 'state' parameter in callback".to_string())
        })?;

        let redirect_uri = params.get("redirect_uri").ok_or_else(|| {
            Error::Validation("Missing 'redirect_uri' parameter in callback".to_string())
        })?;

        tracing::warn!(
            "Callback received with state: {}. Retrieve code_verifier from session storage.",
            state
        );

        let code_verifier = "PLACEHOLDER_CODE_VERIFIER";

        let token_response = self
            .exchange_code_for_tokens(code, redirect_uri, code_verifier)
            .await?;

        let user_info = self.fetch_user_info(&token_response.access_token).await?;

        self.map_user_info(user_info)
    }

    fn supports(&self, credentials: &AuthCredentials) -> bool {
        matches!(
            credentials,
            AuthCredentials::OAuth2Code { .. } | AuthCredentials::OAuth2RefreshToken { .. }
        )
    }
}

/// Resolve OIDC endpoints from discovery or manual config.
async fn resolve_endpoints(config: &AuthProviderConfig) -> Result<(String, String, String)> {
    if let Some(issuer_url) = &config.issuer_url {
        // Try OIDC discovery
        match OidcStrategy::discover_endpoints(issuer_url).await {
            Ok(discovery) => Ok((
                discovery.authorization_endpoint,
                discovery.token_endpoint,
                discovery.userinfo_endpoint,
            )),
            Err(e) => {
                tracing::warn!(
                    "OIDC discovery failed for {}: {}. Falling back to manual configuration.",
                    issuer_url,
                    e
                );
                extract_manual_endpoints(config)
            }
        }
    } else {
        extract_manual_endpoints(config)
    }
}

/// Extract endpoints from manual config fields.
fn extract_manual_endpoints(config: &AuthProviderConfig) -> Result<(String, String, String)> {
    let auth_endpoint = config.authorization_url.as_ref().ok_or_else(|| {
        Error::Validation("OIDC provider requires issuer_url or authorization_url".to_string())
    })?;

    let token_endpoint = config
        .token_url
        .as_ref()
        .ok_or_else(|| Error::Validation("OIDC provider requires token_url".to_string()))?;

    let userinfo_endpoint = config
        .userinfo_url
        .as_ref()
        .ok_or_else(|| Error::Validation("OIDC provider requires userinfo_url".to_string()))?;

    Ok((
        auth_endpoint.clone(),
        token_endpoint.clone(),
        userinfo_endpoint.clone(),
    ))
}
