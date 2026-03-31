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

//! OIDC discovery, token exchange, and user info fetching.

use raisin_error::{Error, Result};
use std::collections::HashMap;

use super::config::{DiscoveryDocument, TokenResponse};
use super::OidcStrategy;

impl OidcStrategy {
    /// Exchange authorization code for tokens (skeleton implementation).
    ///
    /// # Note
    ///
    /// This is a skeleton implementation. When the `oidc` feature is enabled,
    /// this would make an HTTP POST request to the token endpoint.
    ///
    /// # Arguments
    ///
    /// * `code` - Authorization code from the callback
    /// * `redirect_uri` - The same redirect URI used in authorization
    /// * `code_verifier` - The PKCE code verifier
    #[cfg(not(feature = "oidc"))]
    pub(super) async fn exchange_code_for_tokens(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse> {
        let _ = (code, redirect_uri, code_verifier);
        Err(Error::invalid_state(
            "OIDC feature not enabled - cannot exchange authorization code. \
             Enable the 'oidc' feature flag to use real HTTP requests.",
        ))
    }

    /// Exchange authorization code for tokens (real implementation with reqwest).
    ///
    /// Makes an HTTP POST request to the token endpoint with the authorization
    /// code and PKCE verifier.
    #[cfg(feature = "oidc")]
    pub(super) async fn exchange_code_for_tokens(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse> {
        let config = self.get_config()?;

        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("redirect_uri", redirect_uri);
        params.insert("client_id", &config.client_id);
        params.insert("client_secret", &config.client_secret);
        params.insert("code_verifier", code_verifier);

        let client = reqwest::Client::new();
        let response = client
            .post(&config.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::internal(format!("Token exchange request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Unauthorized(format!(
                "Token exchange failed with status {}: {}",
                status, body
            )));
        }

        response
            .json::<TokenResponse>()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse token response: {}", e)))
    }

    /// Fetch user info from the userinfo endpoint (skeleton implementation).
    ///
    /// # Note
    ///
    /// This is a skeleton implementation. When the `oidc` feature is enabled,
    /// this would make an HTTP GET request to the userinfo endpoint.
    #[cfg(not(feature = "oidc"))]
    pub(super) async fn fetch_user_info(
        &self,
        access_token: &str,
    ) -> Result<HashMap<String, serde_json::Value>> {
        let _ = access_token;
        Err(Error::invalid_state(
            "OIDC feature not enabled - cannot fetch user info. \
             Enable the 'oidc' feature flag to use real HTTP requests.",
        ))
    }

    /// Fetch user info from the userinfo endpoint (real implementation with reqwest).
    ///
    /// Makes an HTTP GET request to the userinfo endpoint with the access token.
    #[cfg(feature = "oidc")]
    pub(super) async fn fetch_user_info(
        &self,
        access_token: &str,
    ) -> Result<HashMap<String, serde_json::Value>> {
        let config = self.get_config()?;

        let client = reqwest::Client::new();
        let response = client
            .get(&config.userinfo_endpoint)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::internal(format!("Userinfo request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Unauthorized(format!(
                "Userinfo request failed with status {}: {}",
                status, body
            )));
        }

        response
            .json()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse userinfo response: {}", e)))
    }

    /// Discover OIDC endpoints from issuer URL (skeleton implementation).
    ///
    /// # Note
    ///
    /// This is a skeleton implementation. When the `oidc` feature is enabled,
    /// this would fetch the `.well-known/openid-configuration` document.
    #[cfg(not(feature = "oidc"))]
    pub(super) async fn discover_endpoints(issuer_url: &str) -> Result<DiscoveryDocument> {
        let _ = issuer_url;
        Err(Error::invalid_state(
            "OIDC feature not enabled - cannot perform discovery. \
             Enable the 'oidc' feature flag or provide manual endpoint configuration.",
        ))
    }

    /// Discover OIDC endpoints from issuer URL (real implementation with reqwest).
    ///
    /// Fetches the OpenID Connect discovery document from
    /// `{issuer}/.well-known/openid-configuration`.
    #[cfg(feature = "oidc")]
    pub(super) async fn discover_endpoints(issuer_url: &str) -> Result<DiscoveryDocument> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            issuer_url.trim_end_matches('/')
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| Error::internal(format!("OIDC discovery request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(Error::Validation(format!(
                "OIDC discovery failed with status {}",
                response.status()
            )));
        }

        response
            .json()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse discovery document: {}", e)))
    }
}
