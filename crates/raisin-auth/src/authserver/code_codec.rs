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

//! Self-contained authorization codes.
//!
//! The code handed to the client **carries its own grant**, sealed with
//! AES-256-GCM, instead of naming a row in a store. Any node holding the same
//! master key can verify it, so the authorization server needs no shared state
//! for codes at all.
//!
//! # Why not store them
//!
//! `/authorize` is called by the resource owner's browser; `/token` is called by
//! the MCP host's backend (ChatGPT's servers, not the user's machine). Those are
//! different network origins, so a load balancer in front of a cluster routes
//! them to different nodes as a matter of course. A stored code would then have
//! to reach the second node within the few hundred milliseconds of a redirect —
//! racing asynchronous replication for a record that lives five minutes and is
//! used once. Losing that race produces exactly the intermittent `invalid_grant`
//! that makes a connector feel unreliable. A sealed code cannot lose it.
//!
//! # Sealed, not signed
//!
//! The payload holds the resource owner's identity id and email, and the code
//! travels in a redirect URL — through the browser, the MCP host's servers, and
//! anything logging either. A signed-but-readable token (a bare JWT) would leak
//! all of that to every hop; the opaque random code it replaces leaked nothing.
//! So the payload is *encrypted*, and AES-GCM's authentication tag doubles as
//! the integrity check: tampering fails decryption.
//!
//! # Single use is enforced elsewhere
//!
//! Sealing makes a code verifiable, not un-replayable — a captured code stays
//! valid until it expires. [`AuthorizationCode::code`] is a random jti carried
//! inside the sealed payload for exactly that reason: the authorization server
//! claims a lock on it at redemption and never releases it, so the lease *is*
//! the "already used" marker and it expires with the code. See
//! [`AuthorizationServer::redeem_authorization_code`](super::AuthorizationServer::redeem_authorization_code).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use raisin_crypto::SecretBox;

use super::error::{AuthServerError, AuthServerResult};
use super::model::AuthorizationCode;

/// Seals an [`AuthorizationCode`] into the opaque string given to the client,
/// and opens it again at the token endpoint.
///
/// A trait so tests can substitute a transparent codec, and so a deployment
/// could swap the sealing scheme without touching the protocol logic.
pub trait AuthorizationCodeCodec: Send + Sync {
    /// Seal a code into the value handed to the client.
    fn seal(&self, code: &AuthorizationCode) -> AuthServerResult<String>;

    /// Open a code presented at the token endpoint.
    ///
    /// Returns [`AuthServerError::InvalidGrant`] for anything unreadable: a
    /// tampered payload, one sealed under a different key, or plain garbage. A
    /// caller must not be able to tell those apart.
    fn open(&self, value: &str) -> AuthServerResult<AuthorizationCode>;
}

/// The production codec: AES-256-GCM via [`raisin_crypto::SecretBox`], the same
/// primitive that protects stored provider credentials and connector OAuth
/// tokens.
///
/// **Every node must be given the same master key.** A code sealed on node A is
/// undecryptable on node B otherwise, which presents as a login that always
/// fails at the token step.
pub struct SealedCodeCodec {
    secret_box: SecretBox,
}

impl SealedCodeCodec {
    /// Build a codec over a 32-byte master key.
    pub fn new(master_key: &[u8; 32]) -> Self {
        Self {
            secret_box: SecretBox::new(master_key),
        }
    }

    /// Build a codec over a random per-process key.
    ///
    /// For dev servers and tests only. Codes sealed with it die with the
    /// process and are meaningless to any other node, so a deployment that
    /// restarts breaks in-flight logins and a cluster breaks every token
    /// exchange that crosses a node. Callers should say so in a log line.
    pub fn ephemeral() -> Self {
        use rand::Rng;
        Self::new(&rand::thread_rng().gen())
    }
}

impl AuthorizationCodeCodec for SealedCodeCodec {
    fn seal(&self, code: &AuthorizationCode) -> AuthServerResult<String> {
        let json = serde_json::to_vec(code).map_err(|e| {
            AuthServerError::ServerError(format!("authorization code could not be encoded: {e}"))
        })?;
        let sealed = self.secret_box.encrypt_bytes(&json).map_err(|e| {
            AuthServerError::ServerError(format!("authorization code could not be sealed: {e}"))
        })?;
        Ok(URL_SAFE_NO_PAD.encode(sealed))
    }

    fn open(&self, value: &str) -> AuthServerResult<AuthorizationCode> {
        // Every failure below is reported identically: an attacker probing the
        // endpoint must not learn whether a code was malformed, forged, or
        // simply sealed under another key.
        let invalid =
            || AuthServerError::InvalidGrant("authorization code is not valid".to_string());

        let sealed = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
        let json = self
            .secret_box
            .decrypt_bytes(&sealed)
            .map_err(|_| invalid())?;
        serde_json::from_slice(&json).map_err(|_| invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authserver::pkce::CodeChallengeMethod;

    fn code() -> AuthorizationCode {
        AuthorizationCode {
            code: "jti-1".to_string(),
            client_id: "client-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            redirect_uri: "https://chatgpt.com/connector/oauth/abc".to_string(),
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_string(),
            code_challenge_method: CodeChallengeMethod::S256,
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "studio".to_string(),
            branch: "main".to_string(),
            resource: "https://db.example.com/mcp/studio/main/studio".to_string(),
            scope: "reader".to_string(),
            expires_at: 1_800_000_000,
        }
    }

    #[test]
    fn round_trips_through_an_independent_codec_with_the_same_key() {
        let key = [7u8; 32];
        // Two codecs built separately model two nodes sharing a master key.
        let issuer = SealedCodeCodec::new(&key);
        let redeemer = SealedCodeCodec::new(&key);

        let sealed = issuer.seal(&code()).unwrap();
        let opened = redeemer.open(&sealed).unwrap();

        assert_eq!(opened.code, "jti-1");
        assert_eq!(opened.identity_id, "id-1");
        assert_eq!(opened.scope, "reader");
        assert_eq!(opened.expires_at, 1_800_000_000);
    }

    /// The payload must not be readable in transit — it carries the resource
    /// owner's email through a redirect URL.
    #[test]
    fn sealed_code_does_not_expose_its_payload() {
        let sealed = SealedCodeCodec::new(&[7u8; 32]).seal(&code()).unwrap();
        assert!(!sealed.contains("u@example.com"));
        assert!(!sealed.contains("id-1"));
        // Nor after an obvious decode attempt.
        let raw = URL_SAFE_NO_PAD.decode(&sealed).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("u@example.com"));
    }

    #[test]
    fn a_code_sealed_under_another_key_is_rejected() {
        let sealed = SealedCodeCodec::new(&[7u8; 32]).seal(&code()).unwrap();
        let err = SealedCodeCodec::new(&[8u8; 32])
            .open(&sealed)
            .expect_err("foreign key must not open");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn a_tampered_code_is_rejected() {
        let codec = SealedCodeCodec::new(&[7u8; 32]);
        let sealed = codec.seal(&code()).unwrap();

        let mut bytes = URL_SAFE_NO_PAD.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let tampered = URL_SAFE_NO_PAD.encode(&bytes);

        let err = codec
            .open(&tampered)
            .expect_err("AES-GCM must reject a modified payload");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn garbage_is_rejected_without_panicking() {
        let codec = SealedCodeCodec::new(&[7u8; 32]);
        for bad in ["", "not-base64!!", "aGVsbG8"] {
            assert_eq!(codec.open(bad).unwrap_err().code(), "invalid_grant");
        }
    }
}
