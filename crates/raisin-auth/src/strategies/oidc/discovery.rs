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

//! OIDC discovery, JWKS retrieval, token exchange and user info fetching.
//!
//! # Why the two caches are process-wide
//!
//! A discovery document and a key set belong to the *provider*, not to a
//! tenant, so two tenants configured against the same Keycloak realm should
//! share one fetch. They are keyed by URL for that reason. Both are plain
//! in-memory maps: nothing here is authoritative state, so a cold cache on a
//! restart or on a second cluster node costs one extra HTTP request and
//! nothing else.

use raisin_error::{Error, Result};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::config::{DiscoveryDocument, JwkSet, TokenResponse};
use super::OidcStrategy;

/// How long a discovery document is reused before being fetched again.
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);

/// How long a key set is reused. Shorter than discovery because signing keys
/// rotate; a `kid` miss also forces a refresh regardless of age, so this bound
/// only governs how quickly a *withdrawn* key stops being trusted.
const JWKS_TTL: Duration = Duration::from_secs(600);

/// How long any single provider request may take.
///
/// Every one of these sits inside a user's login redirect, so an unresponsive
/// provider must fail the login rather than hold the connection open.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

type Cache<T> = OnceLock<Mutex<HashMap<String, (Instant, T)>>>;

static DISCOVERY_CACHE: Cache<DiscoveryDocument> = OnceLock::new();
static JWKS_CACHE: Cache<JwkSet> = OnceLock::new();

fn cache_get<T: Clone>(cache: &Cache<T>, key: &str, ttl: Duration) -> Option<T> {
    let map = cache.get_or_init(Default::default).lock().ok()?;
    map.get(key)
        .filter(|(at, _)| at.elapsed() < ttl)
        .map(|(_, v)| v.clone())
}

fn cache_put<T>(cache: &Cache<T>, key: &str, value: &T)
where
    T: Clone,
{
    if let Ok(mut map) = cache.get_or_init(Default::default).lock() {
        map.insert(key.to_string(), (Instant::now(), value.clone()));
    }
}

/// Build the shared HTTP client used for every provider call.
fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| Error::internal(format!("could not build the OIDC HTTP client: {e}")))
}

impl OidcStrategy {
    /// Exchange an authorization code for tokens.
    ///
    /// The `code_verifier` is the PKCE secret minted when the login started; it
    /// is what proves this exchange belongs to the same browser that was sent
    /// to the provider.
    pub(super) async fn exchange_code_for_tokens(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse> {
        let config = self.get_config()?;

        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("redirect_uri", config.redirect_uri.as_str());
        params.insert("client_id", &config.client_id);
        params.insert("code_verifier", code_verifier);
        // A confidential client authenticates with its secret. An empty secret
        // means a public client, which PKCE alone protects; sending an empty
        // `client_secret` makes some providers reject the request outright.
        if !config.client_secret.is_empty() {
            params.insert("client_secret", &config.client_secret);
        }

        let response = client()?
            .post(&config.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::internal(format!("Token exchange request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // The body is a provider error document, not user input, but it is
            // still logged rather than returned verbatim to the browser by the
            // caller.
            return Err(Error::Unauthorized(format!(
                "Token exchange failed with status {status}: {body}"
            )));
        }

        response
            .json::<TokenResponse>()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse token response: {e}")))
    }

    /// Fetch user info from the userinfo endpoint.
    pub(super) async fn fetch_user_info(
        &self,
        access_token: &str,
    ) -> Result<HashMap<String, serde_json::Value>> {
        let config = self.get_config()?;
        let endpoint = config.userinfo_endpoint.as_ref().ok_or_else(|| {
            Error::invalid_state("provider publishes no userinfo endpoint".to_string())
        })?;

        let response = client()?
            .get(endpoint)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::internal(format!("Userinfo request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Unauthorized(format!(
                "Userinfo request failed with status {status}: {body}"
            )));
        }

        response
            .json()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse userinfo response: {e}")))
    }

    /// Fetch the discovery document for an issuer, reusing a cached copy.
    ///
    /// The URL is the issuer with `/.well-known/openid-configuration` appended,
    /// per OpenID Connect Discovery 1.0 §4. A trailing slash on the configured
    /// issuer is trimmed first, or the path would contain `//` and several
    /// providers 404 on it.
    pub(super) async fn discover_endpoints(issuer_url: &str) -> Result<DiscoveryDocument> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            issuer_url.trim_end_matches('/')
        );

        if let Some(hit) = cache_get(&DISCOVERY_CACHE, &discovery_url, DISCOVERY_TTL) {
            return Ok(hit);
        }

        let response = client()?
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| Error::internal(format!("OIDC discovery request failed: {e}")))?;

        if !response.status().is_success() {
            return Err(Error::Validation(format!(
                "OIDC discovery failed with status {}",
                response.status()
            )));
        }

        let doc: DiscoveryDocument = response
            .json()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse discovery document: {e}")))?;

        cache_put(&DISCOVERY_CACHE, &discovery_url, &doc);
        Ok(doc)
    }

    /// Fetch a provider's key set, reusing a cached copy unless `force` is set.
    ///
    /// `force` is how key rotation is survived: the verifier calls once with
    /// the cache, and if the token names a `kid` the cached set does not hold,
    /// once more without it. Without that second call a provider that rotated
    /// its keys would reject every login until the TTL expired.
    pub(super) async fn fetch_jwks(jwks_uri: &str, force: bool) -> Result<JwkSet> {
        if !force {
            if let Some(hit) = cache_get(&JWKS_CACHE, jwks_uri, JWKS_TTL) {
                return Ok(hit);
            }
        }

        let response = client()?
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| Error::internal(format!("JWKS request failed: {e}")))?;

        if !response.status().is_success() {
            return Err(Error::Validation(format!(
                "JWKS request failed with status {}",
                response.status()
            )));
        }

        let set: JwkSet = response
            .json()
            .await
            .map_err(|e| Error::internal(format!("Failed to parse JWKS document: {e}")))?;

        cache_put(&JWKS_CACHE, jwks_uri, &set);
        Ok(set)
    }
}
