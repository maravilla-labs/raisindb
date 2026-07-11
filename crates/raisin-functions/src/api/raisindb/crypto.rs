// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native crypto operation implementations for RaisinFunctionApi.
//!
//! `crypto_verify_jwt` is the network-facing half of the JWT verify primitive:
//! it authorizes the JWKS URL against the function's network policy BEFORE any
//! socket is opened, fetches (and briefly caches) the key set, then delegates to
//! the pure verifier in `runtime::crypto`. Tokens and claims are never logged.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::runtime::crypto::{self, JwtVerifyOptions};

/// How long a fetched JWKS is reused before a re-fetch. Short by design: key
/// rotation should be picked up quickly, but a push storm must not hammer the
/// provider's JWKS endpoint.
const JWKS_CACHE_TTL: Duration = Duration::from_secs(300);
/// Timeout for the JWKS fetch (connect + body).
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Process-wide JWKS cache keyed by URL: `url -> (fetched_at, jwks)`.
fn jwks_cache() -> &'static Mutex<HashMap<String, (Instant, Value)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Value)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(url: &str) -> Option<Value> {
    let guard = jwks_cache().lock().ok()?;
    let (at, v) = guard.get(url)?;
    if at.elapsed() < JWKS_CACHE_TTL {
        Some(v.clone())
    } else {
        None
    }
}

fn cache_put(url: &str, jwks: Value) {
    if let Ok(mut guard) = jwks_cache().lock() {
        guard.insert(url.to_string(), (Instant::now(), jwks));
    }
}

impl RaisinFunctionApi {
    /// Verify an RS256/ES256-signed JWT against a JWKS.
    ///
    /// `opts` = `{ jwks_url?, issuer?, audience?, algorithms? }`. When `jwks_url`
    /// is present it is authorized against the network policy and fetched (with a
    /// brief cache); the verification itself is delegated to the pure
    /// `runtime::crypto` verifier. Returns `{ valid, claims?, error? }` and never
    /// errors on an invalid token (an invalid token is `{ valid: false, error }`).
    /// A hard `Err` is reserved for policy denial and unreachable-JWKS cases.
    pub(crate) async fn impl_crypto_verify_jwt(&self, token: &str, opts: Value) -> Result<Value> {
        let parsed = JwtVerifyOptions::from_value(&opts);

        let jwks_url = opts
            .get("jwks_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let jwks_url = match jwks_url {
            Some(u) => u,
            None => {
                return Ok(serde_json::json!({
                    "valid": false,
                    "error": "jwks_url is required",
                }));
            }
        };

        // NETWORK POLICY: refuse before opening any socket if the JWKS host is
        // not allow-listed — same gate as raisin.imap / raisin.http.
        if !self.is_url_allowed(jwks_url) {
            tracing::debug!(target = %jwks_url, "jwks fetch blocked by network policy");
            return Err(raisin_error::Error::PermissionDenied(format!(
                "[crypto:policy_denied] JWKS endpoint not allowed by network policy: {jwks_url}"
            )));
        }

        let jwks = self.fetch_jwks(jwks_url).await?;
        // Never log token/claims; verification is pure and offline from here.
        Ok(crypto::verify_with_jwks(token, &jwks, &parsed))
    }

    /// Fetch the JWKS document, serving a cached copy while it is fresh.
    async fn fetch_jwks(&self, url: &str) -> Result<Value> {
        if let Some(cached) = cache_get(url) {
            return Ok(cached);
        }
        // Disable redirects: the `is_url_allowed` gate only vets the initial
        // URL, so a redirect (e.g. to 169.254.169.254 or an RFC1918 host) would
        // bypass the network policy — a classic SSRF. Legitimate JWKS endpoints
        // serve the document directly; a redirect here fails closed.
        let client = reqwest::Client::builder()
            .timeout(JWKS_FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| raisin_error::Error::Backend(format!("jwks client: {e}")))?;
        let resp =
            client.get(url).send().await.map_err(|e| {
                raisin_error::Error::Backend(format!("[crypto:jwks_unreachable] {e}"))
            })?;
        if !resp.status().is_success() {
            return Err(raisin_error::Error::Backend(format!(
                "[crypto:jwks_unreachable] JWKS endpoint returned {}",
                resp.status()
            )));
        }
        let jwks: Value = resp
            .json()
            .await
            .map_err(|e| raisin_error::Error::Backend(format!("[crypto:jwks_invalid] {e}")))?;
        cache_put(url, jwks.clone());
        Ok(jwks)
    }
}
