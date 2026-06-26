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

//! Inbound PKCE verification for the authorization-code grant (RFC 7636).
//!
//! The challenge generation primitives already live on
//! [`OidcStrategy`](crate::strategies::OidcStrategy) for the outbound OIDC
//! client flow. Here we add the *verification* direction the authorization
//! server needs: recompute the challenge from the verifier the client presents
//! at the token endpoint and compare it, in constant time, against the
//! challenge captured at the authorization endpoint.

use serde::{Deserialize, Serialize};

use crate::strategies::OidcStrategy;

use super::error::{AuthServerError, AuthServerResult};

/// The PKCE code-challenge transformation a client used (RFC 7636 §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CodeChallengeMethod {
    /// SHA-256 of the verifier, base64url-encoded. The only method OAuth 2.1
    /// permits a public client to use.
    S256,
}

impl CodeChallengeMethod {
    /// Parse the `code_challenge_method` query parameter.
    ///
    /// OAuth 2.1 forbids `plain`; an absent method defaults to `S256` only when
    /// a challenge is present, which the caller decides. Any value other than
    /// `S256` is rejected.
    pub fn parse(raw: &str) -> AuthServerResult<Self> {
        match raw {
            "S256" => Ok(Self::S256),
            "plain" => Err(AuthServerError::InvalidRequest(
                "code_challenge_method 'plain' is not permitted; use S256".to_string(),
            )),
            other => Err(AuthServerError::InvalidRequest(format!(
                "unsupported code_challenge_method: {other}"
            ))),
        }
    }
}

impl std::fmt::Display for CodeChallengeMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::S256 => f.write_str("S256"),
        }
    }
}

/// Validate the shape of a PKCE `code_challenge` per RFC 7636 §4.2.
///
/// A base64url(SHA-256(...)) value is always 43 characters. We accept the
/// RFC's broader 43..=128 range to remain lenient about client encodings while
/// still rejecting empty or oversized inputs.
pub fn validate_code_challenge(challenge: &str) -> AuthServerResult<()> {
    let len = challenge.len();
    if !(43..=128).contains(&len) {
        return Err(AuthServerError::InvalidRequest(format!(
            "code_challenge must be 43-128 characters, got {len}"
        )));
    }
    if !challenge
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~')
    {
        return Err(AuthServerError::InvalidRequest(
            "code_challenge contains characters outside the unreserved set".to_string(),
        ));
    }
    Ok(())
}

/// Verify a presented `code_verifier` against the stored `code_challenge`.
///
/// Recomputes `BASE64URL(SHA256(verifier))` using the shared challenge
/// generator and compares it to `challenge` in constant time. Returns
/// [`AuthServerError::InvalidGrant`] on any mismatch, matching the error the
/// token endpoint must surface for a failed PKCE check (RFC 7636 §4.6).
pub fn verify_pkce(
    method: CodeChallengeMethod,
    challenge: &str,
    verifier: &str,
) -> AuthServerResult<()> {
    // RFC 7636 §4.1: the verifier is 43-128 chars from the unreserved set.
    let vlen = verifier.len();
    if !(43..=128).contains(&vlen) {
        return Err(AuthServerError::InvalidGrant(format!(
            "code_verifier must be 43-128 characters, got {vlen}"
        )));
    }

    let computed = match method {
        CodeChallengeMethod::S256 => OidcStrategy::generate_code_challenge(verifier),
    };

    if constant_time_eq(computed.as_bytes(), challenge.as_bytes()) {
        Ok(())
    } else {
        Err(AuthServerError::InvalidGrant(
            "PKCE verification failed: code_verifier does not match code_challenge".to_string(),
        ))
    }
}

/// Length-independent constant-time byte comparison.
///
/// Folds the length difference into the accumulator so neither the contents nor
/// the lengths leak through timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u8;
    let n = a.len().min(b.len());
    for i in 0..n {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategies::OidcStrategy;

    #[test]
    fn verify_pkce_accepts_matching_verifier() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);

        verify_pkce(CodeChallengeMethod::S256, &challenge, &verifier)
            .expect("matching verifier should pass");
    }

    #[test]
    fn verify_pkce_rejects_wrong_verifier() {
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let other = OidcStrategy::generate_code_verifier();

        let err = verify_pkce(CodeChallengeMethod::S256, &challenge, &other)
            .expect_err("non-matching verifier must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn verify_pkce_rejects_short_verifier() {
        let challenge = OidcStrategy::generate_code_challenge("anything");
        let err = verify_pkce(CodeChallengeMethod::S256, &challenge, "tooshort")
            .expect_err("short verifier must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[test]
    fn method_rejects_plain() {
        let err = CodeChallengeMethod::parse("plain").expect_err("plain must be rejected");
        assert_eq!(err.code(), "invalid_request");
    }

    #[test]
    fn challenge_shape_validation() {
        // 43-char base64url is the canonical S256 challenge length.
        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        validate_code_challenge(&challenge).expect("canonical challenge should validate");

        validate_code_challenge("short").expect_err("short challenge must fail");
    }

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
