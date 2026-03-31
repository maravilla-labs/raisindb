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

//! PKCE (Proof Key for Code Exchange) and authorization URL construction.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use raisin_error::{Error, Result};
use sha2::{Digest, Sha256};
use url::Url;

use super::OidcStrategy;

impl OidcStrategy {
    /// Generate a PKCE code verifier.
    ///
    /// Returns a cryptographically secure random string suitable for use as
    /// a PKCE code verifier (43-128 characters).
    ///
    /// # Returns
    ///
    /// A base64url-encoded random string.
    pub fn generate_code_verifier() -> String {
        use rand::Rng;
        let random_bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen()).collect();
        URL_SAFE_NO_PAD.encode(&random_bytes)
    }

    /// Generate a PKCE code challenge from a verifier.
    ///
    /// Uses SHA256 hashing as specified by RFC 7636.
    ///
    /// # Arguments
    ///
    /// * `verifier` - The code verifier
    ///
    /// # Returns
    ///
    /// The base64url-encoded SHA256 hash of the verifier.
    pub fn generate_code_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        URL_SAFE_NO_PAD.encode(hash)
    }

    /// Build the authorization URL with all required parameters.
    ///
    /// # Arguments
    ///
    /// * `redirect_uri` - The callback URL where the provider will redirect
    /// * `state` - CSRF protection token
    /// * `code_challenge` - PKCE code challenge
    /// * `nonce` - Optional nonce for ID token validation
    pub(super) fn build_authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
        nonce: Option<&str>,
    ) -> Result<String> {
        let config = self.get_config()?;

        let mut url = Url::parse(&config.authorization_endpoint)
            .map_err(|e| Error::internal(format!("Invalid authorization endpoint URL: {}", e)))?;

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", &config.client_id);
            query.append_pair("response_type", "code");
            query.append_pair("redirect_uri", redirect_uri);
            query.append_pair("scope", &config.scopes.join(" "));
            query.append_pair("state", state);
            query.append_pair("code_challenge", code_challenge);
            query.append_pair("code_challenge_method", "S256");

            if let Some(nonce) = nonce {
                query.append_pair("nonce", nonce);
            }
        }

        Ok(url.to_string())
    }
}
