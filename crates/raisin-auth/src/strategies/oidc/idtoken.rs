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

//! `id_token` verification.
//!
//! The `id_token` is the only part of the OIDC response that is *proof* of
//! anything. The authorization code proves the browser reached the provider,
//! and the userinfo response proves only that we hold an access token; the
//! signed token is what binds a subject to this client at this issuer. So the
//! subject and the verified email used to resolve an account come from here,
//! never from the userinfo document.
//!
//! Five checks, all mandatory:
//!
//! - **signature**, against a key from the provider's JWKS,
//! - **issuer**, exactly the `issuer` the discovery document declared,
//! - **audience**, exactly our `client_id`, so a token minted for another
//!   client of the same provider cannot be replayed at ours,
//! - **expiry**, with a small leeway for clock skew,
//! - **nonce**, matching the one sealed into the login state, which is what
//!   makes a captured token useless in a second login.
//!
//! `alg` is pinned to RS256 rather than read from the token header. Trusting
//! the header is the classic JWT downgrade: a token presented as `alg: none`,
//! or as `HS256` signed with the public key as the HMAC secret, verifies
//! against a naive implementation.

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use raisin_error::{Error, Result};
use std::collections::HashMap;

use super::config::Jwk;
use super::OidcStrategy;

/// Clock skew tolerated on `exp` and `nbf`, in seconds.
const LEEWAY_SECONDS: u64 = 60;

/// The verified claim set of an `id_token`.
pub type VerifiedClaims = HashMap<String, serde_json::Value>;

impl OidcStrategy {
    /// Verify an `id_token` and return its claims.
    ///
    /// `expected_nonce` is the value minted when the login started. It is
    /// required, not optional: a provider that omits the claim, or returns a
    /// different one, fails the login.
    pub(super) async fn verify_id_token(
        &self,
        id_token: &str,
        expected_nonce: &str,
    ) -> Result<VerifiedClaims> {
        let config = self.get_config()?;

        let jwks_uri = config.jwks_uri.as_ref().ok_or_else(|| {
            Error::Validation(
                "provider publishes no jwks_uri, so its id_token signature cannot be verified"
                    .to_string(),
            )
        })?;

        let header = decode_header(id_token)
            .map_err(|e| Error::Unauthorized(format!("id_token header is unreadable: {e}")))?;
        if header.alg != Algorithm::RS256 {
            return Err(Error::Unauthorized(format!(
                "id_token is signed with {:?}; only RS256 is accepted",
                header.alg
            )));
        }

        // First pass over the cached key set, then one forced refetch if the
        // token names a key we have not seen. That second call is what carries
        // a deployment through a provider's key rotation.
        let mut set = Self::fetch_jwks(jwks_uri, false).await?;
        if select_key(&set.keys, header.kid.as_deref()).is_none() {
            set = Self::fetch_jwks(jwks_uri, true).await?;
        }
        let jwk = select_key(&set.keys, header.kid.as_deref()).ok_or_else(|| {
            Error::Unauthorized(
                "id_token was signed with a key absent from the provider's JWKS".to_string(),
            )
        })?;

        let key = DecodingKey::from_rsa_components(
            jwk.n.as_deref().unwrap_or_default(),
            jwk.e.as_deref().unwrap_or_default(),
        )
        .map_err(|e| Error::internal(format!("provider JWKS key is malformed: {e}")))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = LEEWAY_SECONDS;
        validation.set_issuer(&[config.issuer.as_str()]);
        validation.set_audience(&[config.client_id.as_str()]);
        validation.validate_exp = true;

        let data = decode::<VerifiedClaims>(id_token, &key, &validation)
            .map_err(|e| Error::Unauthorized(format!("id_token is not valid: {e}")))?;
        let claims = data.claims;

        verify_nonce(&claims, expected_nonce)?;
        verify_azp(&claims, &config.client_id)?;

        Ok(claims)
    }

    /// Reject an email whose domain is outside the provider's allow list.
    ///
    /// Empty list means every domain is allowed. The comparison is
    /// case-insensitive on the domain only: the local part of an address is
    /// case-sensitive per RFC 5321, and we are not matching it here anyway.
    pub(super) fn check_email_domain(&self, email: Option<&str>) -> Result<()> {
        let config = self.get_config()?;
        if config.allowed_email_domains.is_empty() {
            return Ok(());
        }

        let domain = email
            .and_then(|e| e.rsplit_once('@'))
            .map(|(_, d)| d.to_ascii_lowercase())
            .ok_or_else(|| {
                Error::Unauthorized(
                    "this provider restricts sign-in by email domain, but the token carries no \
                     email address"
                        .to_string(),
                )
            })?;

        if config
            .allowed_email_domains
            .iter()
            .any(|allowed| allowed.trim().to_ascii_lowercase() == domain)
        {
            Ok(())
        } else {
            Err(Error::Unauthorized(format!(
                "the email domain '{domain}' is not permitted to sign in through this provider"
            )))
        }
    }
}

/// Pick the verification key for a token.
///
/// With a `kid` we take that exact key. Without one we accept a set holding
/// exactly one RSA key, which is what a small Keycloak realm publishes; a set
/// with several keys and no `kid` is ambiguous and rejected rather than
/// guessed at.
fn select_key<'a>(keys: &'a [Jwk], kid: Option<&str>) -> Option<&'a Jwk> {
    let rsa: Vec<&Jwk> = keys
        .iter()
        .filter(|k| k.is_rsa() && k.alg.as_deref().is_none_or(|a| a == "RS256"))
        .collect();

    match kid {
        Some(kid) => rsa
            .into_iter()
            .find(|k| k.kid.as_deref() == Some(kid))
            .or(None),
        None if rsa.len() == 1 => Some(rsa[0]),
        None => None,
    }
}

/// Compare the token's `nonce` against the one this login minted.
fn verify_nonce(claims: &VerifiedClaims, expected: &str) -> Result<()> {
    let present = claims.get("nonce").and_then(|v| v.as_str());
    match present {
        Some(n) if n == expected => Ok(()),
        Some(_) => Err(Error::Unauthorized(
            "id_token nonce does not match this login attempt".to_string(),
        )),
        None => Err(Error::Unauthorized(
            "id_token carries no nonce, so it cannot be bound to this login attempt".to_string(),
        )),
    }
}

/// When `azp` is present it must name us.
///
/// A token whose `aud` holds several clients is only ours if `azp` says so
/// (OpenID Connect Core §3.1.3.7). Checking audience alone would let a token
/// issued for a different party through whenever the provider lists us
/// alongside it.
fn verify_azp(claims: &VerifiedClaims, client_id: &str) -> Result<()> {
    match claims.get("azp").and_then(|v| v.as_str()) {
        Some(azp) if azp != client_id => Err(Error::Unauthorized(
            "id_token was authorized for a different party".to_string(),
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rsa(kid: &str) -> Jwk {
        Jwk {
            kty: Some("RSA".to_string()),
            kid: Some(kid.to_string()),
            alg: Some("RS256".to_string()),
            n: Some("n".to_string()),
            e: Some("AQAB".to_string()),
        }
    }

    #[test]
    fn a_named_kid_selects_that_key() {
        let keys = vec![rsa("a"), rsa("b")];
        assert_eq!(
            select_key(&keys, Some("b")).unwrap().kid.as_deref(),
            Some("b")
        );
    }

    #[test]
    fn an_unknown_kid_selects_nothing() {
        let keys = vec![rsa("a")];
        assert!(select_key(&keys, Some("zzz")).is_none());
    }

    /// A lone key is unambiguous, so a token without `kid` still verifies.
    /// This is what a default Keycloak realm looks like.
    #[test]
    fn a_single_key_is_used_when_the_token_names_none() {
        let keys = vec![rsa("only")];
        assert!(select_key(&keys, None).is_some());
    }

    /// Several keys and no `kid` is ambiguous. Picking the first would make
    /// verification depend on the provider's listing order.
    #[test]
    fn several_keys_and_no_kid_is_refused() {
        let keys = vec![rsa("a"), rsa("b")];
        assert!(select_key(&keys, None).is_none());
    }

    /// An EC key in the set must be skipped, not decoded as RSA.
    #[test]
    fn non_rsa_keys_are_filtered_out() {
        let ec = Jwk {
            kty: Some("EC".to_string()),
            kid: Some("ec".to_string()),
            alg: None,
            n: None,
            e: None,
        };
        assert!(select_key(&[ec], Some("ec")).is_none());
    }

    #[test]
    fn nonce_must_match() {
        let mut claims = VerifiedClaims::new();
        claims.insert("nonce".to_string(), json!("abc"));
        assert!(verify_nonce(&claims, "abc").is_ok());
        assert!(verify_nonce(&claims, "def").is_err());
    }

    /// The check a replayed token has to fail. A provider that omits the claim
    /// entirely must not be treated as a pass.
    #[test]
    fn a_missing_nonce_is_refused() {
        assert!(verify_nonce(&VerifiedClaims::new(), "abc").is_err());
    }

    #[test]
    fn azp_naming_another_client_is_refused() {
        let mut claims = VerifiedClaims::new();
        claims.insert("azp".to_string(), json!("someone-else"));
        assert!(verify_azp(&claims, "us").is_err());

        claims.insert("azp".to_string(), json!("us"));
        assert!(verify_azp(&claims, "us").is_ok());
        // Absent azp is fine: the audience check already covers it.
        assert!(verify_azp(&VerifiedClaims::new(), "us").is_ok());
    }
}
