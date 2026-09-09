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

//! The in-flight state of one browser login, sealed into the `state` parameter.
//!
//! # Why the state carries itself
//!
//! Between `/auth/oidc/{provider}` and its callback the server must remember a
//! PKCE code verifier, a nonce, and where to send the browser afterwards. The
//! obvious home is a table row keyed by an opaque `state` string.
//!
//! Both halves are browser redirects, so on a cluster behind a load balancer
//! they routinely land on different nodes. A stored row would have to replicate
//! between them within the few hundred milliseconds a user spends at the
//! provider, for a record that lives ten minutes and is read once. Losing that
//! race is an intermittent "invalid state" that only appears under load. This
//! is the same reasoning, and the same primitive, as the authorization-code
//! codec in [`crate::authserver::code_codec`].
//!
//! So the state is *sealed*, not stored: AES-256-GCM under the deployment's
//! master key. Any node holding that key can open it and no one else can.
//!
//! # Sealed rather than signed
//!
//! The payload holds the PKCE verifier. A signed-but-readable token would put
//! the verifier in a URL that travels through the browser, the provider's
//! servers, and the logs of everything between, which defeats the point of
//! PKCE entirely. Encrypting it means the parameter is opaque to every hop.
//!
//! # What it does not do
//!
//! Sealing makes the state verifiable, not single-use. Replay is bounded two
//! ways instead: the state expires in [`LOGIN_STATE_TTL_SECONDS`], and the
//! authorization code inside a replayed callback is single-use at the provider.
//! The nonce is the binding that matters — an `id_token` obtained in one login
//! will not verify against another login's state.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use raisin_crypto::{SecretBox, SecretContext};
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};

/// How long a login may sit at the provider before its state stops opening.
///
/// Long enough for a password manager, a second factor and a consent screen;
/// short enough that a state captured from a browser history is stale.
pub const LOGIN_STATE_TTL_SECONDS: i64 = 600;

/// Everything the callback needs to finish a login it did not start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OidcLoginState {
    /// Provider slug, matching the `{provider}` path segment. Checked at the
    /// callback so a state minted for one provider cannot be presented at
    /// another's endpoint.
    pub provider: String,

    /// Tenant the login belongs to. Checked for the same reason: a tenant is
    /// resolved from the request host, and a state must not cross between two.
    pub tenant_id: String,

    /// PKCE code verifier. The secret half of the challenge sent to the
    /// provider.
    pub code_verifier: String,

    /// Nonce echoed in the `id_token`, binding that token to this login.
    pub nonce: String,

    /// Where to send the browser once tokens exist. `None` means answer with
    /// JSON instead of redirecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,

    /// Repository to provision the user into, when the login named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,

    /// Absolute expiry, Unix seconds.
    pub expires_at: i64,
}

impl OidcLoginState {
    /// Whether this state is past its expiry.
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() >= self.expires_at
    }

    /// Seal into the opaque value carried as the `state` query parameter.
    pub fn seal(&self, master_key: &[u8; 32]) -> Result<String> {
        let ctx = context(&self.tenant_id)?;
        let json = serde_json::to_vec(self)
            .map_err(|e| Error::internal(format!("login state could not be encoded: {e}")))?;
        let sealed = SecretBox::new(master_key)
            .seal(&ctx, &json)
            .map_err(|e| Error::internal(format!("login state could not be sealed: {e}")))?;
        Ok(URL_SAFE_NO_PAD.encode(sealed))
    }

    /// Open a `state` parameter, checking that it is for this tenant, this
    /// provider, and still current.
    ///
    /// Every failure returns the same message. An endpoint that distinguished
    /// "expired" from "forged" from "another tenant's" would tell a prober
    /// which of those they had achieved.
    pub fn open(
        value: &str,
        master_key: &[u8; 32],
        tenant_id: &str,
        provider: &str,
    ) -> Result<Self> {
        let invalid = || Error::Unauthorized("login state is not valid".to_string());

        let ctx = context(tenant_id)?;
        let sealed = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
        let json = SecretBox::new(master_key)
            .open(&ctx, &sealed)
            .map_err(|_| invalid())?;
        let state: Self = serde_json::from_slice(&json).map_err(|_| invalid())?;

        if state.tenant_id != tenant_id || state.provider != provider || state.is_expired() {
            return Err(invalid());
        }
        Ok(state)
    }
}

fn context(tenant_id: &str) -> Result<SecretContext> {
    SecretContext::oidc_login_state(tenant_id)
        .map_err(|e| Error::internal(format!("login state context is invalid: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> OidcLoginState {
        OidcLoginState {
            provider: "keycloak".to_string(),
            tenant_id: "acme".to_string(),
            code_verifier: "v".repeat(43),
            nonce: "nonce-1".to_string(),
            redirect_uri: Some("https://app.example.com/signed-in".to_string()),
            repo: Some("studio".to_string()),
            expires_at: chrono::Utc::now().timestamp() + LOGIN_STATE_TTL_SECONDS,
        }
    }

    /// Two boxes built separately from one key model two cluster nodes: the
    /// node that starts a login is often not the node that finishes it.
    #[test]
    fn round_trips_across_nodes_sharing_a_key() {
        let key = [3u8; 32];
        let sealed = state().seal(&key).unwrap();
        let opened = OidcLoginState::open(&sealed, &key, "acme", "keycloak").unwrap();
        assert_eq!(opened, state());
    }

    /// The verifier must not be readable in a URL.
    #[test]
    fn the_verifier_is_not_visible_in_the_state_parameter() {
        let s = state();
        let sealed = s.seal(&[3u8; 32]).unwrap();
        assert!(!sealed.contains(&s.code_verifier));
        let raw = URL_SAFE_NO_PAD.decode(&sealed).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains(&s.code_verifier));
    }

    #[test]
    fn a_state_sealed_under_another_key_is_refused() {
        let sealed = state().seal(&[3u8; 32]).unwrap();
        assert!(OidcLoginState::open(&sealed, &[4u8; 32], "acme", "keycloak").is_err());
    }

    #[test]
    fn a_tampered_state_is_refused() {
        let sealed = state().seal(&[3u8; 32]).unwrap();
        let mut bytes = URL_SAFE_NO_PAD.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let tampered = URL_SAFE_NO_PAD.encode(&bytes);
        assert!(OidcLoginState::open(&tampered, &[3u8; 32], "acme", "keycloak").is_err());
    }

    /// A state is bound to the tenant that minted it. Tenants are resolved from
    /// the request host, so without this a state issued on one host would work
    /// on another.
    #[test]
    fn a_state_does_not_cross_tenants() {
        let sealed = state().seal(&[3u8; 32]).unwrap();
        assert!(OidcLoginState::open(&sealed, &[3u8; 32], "othercorp", "keycloak").is_err());
    }

    /// Nor between two providers of the same tenant: a verifier minted for the
    /// staff IdP must not complete a login at the customer one.
    #[test]
    fn a_state_does_not_cross_providers() {
        let sealed = state().seal(&[3u8; 32]).unwrap();
        assert!(OidcLoginState::open(&sealed, &[3u8; 32], "acme", "google").is_err());
    }

    #[test]
    fn an_expired_state_is_refused() {
        let mut s = state();
        s.expires_at = chrono::Utc::now().timestamp() - 1;
        let sealed = s.seal(&[3u8; 32]).unwrap();
        assert!(OidcLoginState::open(&sealed, &[3u8; 32], "acme", "keycloak").is_err());
    }

    #[test]
    fn garbage_is_refused_without_panicking() {
        for bad in ["", "not base64!!", "aGVsbG8"] {
            assert!(OidcLoginState::open(bad, &[3u8; 32], "acme", "keycloak").is_err());
        }
    }
}
