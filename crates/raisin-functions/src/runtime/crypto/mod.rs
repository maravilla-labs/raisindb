// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native JWT / OIDC verification binding for server-side functions.
//!
//! This module owns the (pure, offline) JWT verification logic in Rust and is
//! the crypto sibling of `runtime::imap`: a general native primitive exposed to
//! the QuickJS and Starlark runtimes via the `raisin.crypto.*` namespace. It
//! verifies an RS256/ES256-signed JWT against a JWKS (JSON Web Key Set) and
//! checks the standard temporal (`exp`/`nbf`) and, when requested, `iss`/`aud`
//! claims.
//!
//! It is **not** provider-specific. It happens to be exactly what is needed to
//! verify a Gmail Pub/Sub push (an OIDC bearer JWT), but any connector — shipped
//! or user-authored — can use it to authenticate a signed push/webhook.
//!
//! Security: the JWKS is a network resource. The **fetch** of the JWKS lives in
//! `api/raisindb/crypto.rs`, which authorizes `jwks_url` against the function's
//! `NetworkPolicy` BEFORE any socket is opened. This module is pure: it takes an
//! already-fetched JWKS value and never touches the network. Tokens and claims
//! are never logged.

mod primitives;
pub use primitives::{
    generate_key_pair, hash_hex, random_bytes, random_bytes_check_len, sign_jwt, SignJwtOptions,
};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::{json, Value};
use std::str::FromStr;

/// Parsed, non-secret verification options (minus `jwks_url`, which the API
/// layer consumes to fetch the key set).
#[derive(Debug, Clone, Default)]
pub struct JwtVerifyOptions {
    /// Expected `iss` claim. When set, a mismatching token is rejected.
    pub issuer: Option<String>,
    /// Expected `aud` claim. When set, a mismatching token is rejected; when
    /// unset, audience validation is disabled.
    pub audience: Option<String>,
    /// Allow-list of acceptable signature algorithms (e.g. `["RS256","ES256"]`).
    /// Empty means "accept whatever the token header advertises" (still
    /// constrained to the algorithms this module supports).
    pub algorithms: Vec<String>,
}

impl JwtVerifyOptions {
    /// Extract the verification options from a caller-supplied `opts` object,
    /// ignoring `jwks_url` (handled by the API layer).
    pub fn from_value(opts: &Value) -> Self {
        let issuer = opts
            .get("issuer")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let audience = opts
            .get("audience")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let algorithms = opts
            .get("algorithms")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            issuer,
            audience,
            algorithms,
        }
    }
}

/// Parse a JWS algorithm name into a [`jsonwebtoken::Algorithm`], accepting only
/// the asymmetric algorithms a JWKS-based verify makes sense for.
fn parse_alg(name: &str) -> Option<Algorithm> {
    // jsonwebtoken's FromStr covers all variants; we then reject symmetric
    // (HS*) algorithms because a JWKS verify must be asymmetric.
    match Algorithm::from_str(name).ok()? {
        a @ (Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512
        | Algorithm::ES256
        | Algorithm::ES384
        | Algorithm::EdDSA) => Some(a),
        _ => None,
    }
}

/// Build the failure shape `{ valid: false, error }`. The message is a stable
/// short reason and never contains the token or decoded claims.
fn invalid(reason: impl Into<String>) -> Value {
    json!({ "valid": false, "error": reason.into() })
}

/// Verify `token` against an already-fetched `jwks` (a JWKS document) and the
/// caller's `opts`.
///
/// Returns the public result shape:
/// - `{ "valid": true, "claims": { ... } }` on a fully-validated token, or
/// - `{ "valid": false, "error": "<reason>" }` otherwise.
///
/// This function performs **no** network I/O; the JWKS must already be resolved.
/// It checks signature, `exp`, `nbf`, and (when the corresponding option is set)
/// `iss` and `aud`.
pub fn verify_with_jwks(token: &str, jwks: &Value, opts: &JwtVerifyOptions) -> Value {
    // 1. Decode the (unverified) header to learn the key id + algorithm.
    let header = match decode_header(token) {
        Ok(h) => h,
        Err(_) => return invalid("malformed token header"),
    };

    // 2. Resolve the algorithm, honoring the caller allow-list.
    if !opts.algorithms.is_empty() {
        let alg_name = format!("{:?}", header.alg);
        let allowed = opts
            .algorithms
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&alg_name));
        if !allowed {
            return invalid("algorithm not allowed");
        }
    }
    let alg = match parse_alg(&format!("{:?}", header.alg)) {
        Some(a) => a,
        None => return invalid("unsupported algorithm"),
    };

    // 3. Parse the JWKS and select the signing key by `kid` (or the sole key).
    let set: JwkSet = match serde_json::from_value(jwks.clone()) {
        Ok(s) => s,
        Err(_) => return invalid("invalid jwks"),
    };
    let jwk = match header.kid.as_deref() {
        Some(kid) => set.find(kid),
        None if set.keys.len() == 1 => set.keys.first(),
        None => None,
    };
    let jwk = match jwk {
        Some(j) => j,
        None => return invalid("no matching signing key"),
    };
    let key = match DecodingKey::from_jwk(jwk) {
        Ok(k) => k,
        Err(_) => return invalid("unusable signing key"),
    };

    // 4. Build validation: always check exp + nbf; conditionally iss/aud.
    let mut validation = Validation::new(alg);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    match &opts.audience {
        Some(aud) => validation.set_audience(&[aud]),
        None => validation.validate_aud = false,
    }
    if let Some(iss) = &opts.issuer {
        validation.set_issuer(&[iss]);
    }

    // 5. Verify signature + claims. On success, surface the decoded claims.
    match decode::<Value>(token, &key, &validation) {
        Ok(data) => json!({ "valid": true, "claims": data.claims }),
        Err(e) => invalid(classify_error(&e)),
    }
}

/// Map a `jsonwebtoken` error into a stable, non-sensitive reason string.
fn classify_error(e: &jsonwebtoken::errors::Error) -> &'static str {
    use jsonwebtoken::errors::ErrorKind;
    match e.kind() {
        ErrorKind::InvalidSignature => "invalid signature",
        ErrorKind::ExpiredSignature => "token expired",
        ErrorKind::ImmatureSignature => "token not yet valid",
        ErrorKind::InvalidIssuer => "issuer mismatch",
        ErrorKind::InvalidAudience => "audience mismatch",
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => "algorithm mismatch",
        _ => "verification failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use rsa::pkcs8::EncodePrivateKey;
    use rsa::traits::PublicKeyParts;
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use std::sync::OnceLock;

    /// An ephemeral RSA test keypair generated once per test run — nothing is
    /// committed to the repo. Holds the PKCS#8 PEM (for signing) and the
    /// matching JWK set (for verification), sharing `kid = "test-key-1"`.
    struct TestKey {
        pem: String,
        jwks: Value,
    }

    fn test_key() -> &'static TestKey {
        static KEY: OnceLock<TestKey> = OnceLock::new();
        KEY.get_or_init(|| {
            let mut rng = rand::thread_rng();
            let private = RsaPrivateKey::new(&mut rng, 2048).expect("generate test RSA key");
            let pem = private
                .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
                .expect("encode pkcs8 pem")
                .to_string();
            let public = RsaPublicKey::from(&private);
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
            let n = b64.encode(public.n().to_bytes_be());
            let e = b64.encode(public.e().to_bytes_be());
            let jwks = json!({
                "keys": [{
                    "kty": "RSA", "use": "sig", "alg": "RS256",
                    "kid": "test-key-1", "n": n, "e": e,
                }]
            });
            TestKey { pem, jwks }
        })
    }

    fn make_token(claims: Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());
        let key = EncodingKey::from_rsa_pem(test_key().pem.as_bytes()).unwrap();
        encode(&header, &claims, &key).unwrap()
    }

    fn jwks() -> Value {
        test_key().jwks.clone()
    }

    fn future() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600
    }

    fn past() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 3600
    }

    #[test]
    fn verifies_valid_token() {
        let token = make_token(json!({ "sub": "abc", "exp": future() }));
        let out = verify_with_jwks(&token, &jwks(), &JwtVerifyOptions::default());
        assert_eq!(out["valid"], json!(true), "out={out}");
        assert_eq!(out["claims"]["sub"], json!("abc"));
    }

    #[test]
    fn rejects_expired_token() {
        let token = make_token(json!({ "sub": "abc", "exp": past() }));
        let out = verify_with_jwks(&token, &jwks(), &JwtVerifyOptions::default());
        assert_eq!(out["valid"], json!(false));
        assert_eq!(out["error"], json!("token expired"));
    }

    #[test]
    fn rejects_immature_nbf() {
        let token = make_token(json!({ "sub": "abc", "exp": future(), "nbf": future() }));
        let out = verify_with_jwks(&token, &jwks(), &JwtVerifyOptions::default());
        assert_eq!(out["valid"], json!(false));
        assert_eq!(out["error"], json!("token not yet valid"));
    }

    #[test]
    fn enforces_issuer() {
        let token = make_token(json!({ "sub": "abc", "exp": future(), "iss": "good" }));
        let opts = JwtVerifyOptions {
            issuer: Some("bad".into()),
            ..Default::default()
        };
        let out = verify_with_jwks(&token, &jwks(), &opts);
        assert_eq!(out["valid"], json!(false));
        assert_eq!(out["error"], json!("issuer mismatch"));
    }

    #[test]
    fn enforces_audience() {
        let token = make_token(json!({ "sub": "abc", "exp": future(), "aud": "svc-a" }));
        let ok = verify_with_jwks(
            &token,
            &jwks(),
            &JwtVerifyOptions {
                audience: Some("svc-a".into()),
                ..Default::default()
            },
        );
        assert_eq!(ok["valid"], json!(true));
        let bad = verify_with_jwks(
            &token,
            &jwks(),
            &JwtVerifyOptions {
                audience: Some("svc-b".into()),
                ..Default::default()
            },
        );
        assert_eq!(bad["valid"], json!(false));
        assert_eq!(bad["error"], json!("audience mismatch"));
    }

    #[test]
    fn rejects_disallowed_algorithm() {
        let token = make_token(json!({ "sub": "abc", "exp": future() }));
        let opts = JwtVerifyOptions {
            algorithms: vec!["ES256".into()],
            ..Default::default()
        };
        let out = verify_with_jwks(&token, &jwks(), &opts);
        assert_eq!(out["valid"], json!(false));
        assert_eq!(out["error"], json!("algorithm not allowed"));
    }

    #[test]
    fn rejects_wrong_key() {
        // A JWKS whose modulus differs -> signature cannot verify.
        let mut bad = jwks();
        bad["keys"][0]["n"] = json!("aGVsbG8td29ybGQtYm9ndXMtbW9kdWx1cw");
        let token = make_token(json!({ "sub": "abc", "exp": future() }));
        let out = verify_with_jwks(&token, &bad, &JwtVerifyOptions::default());
        assert_eq!(out["valid"], json!(false));
    }

    #[test]
    fn options_parse_from_value() {
        let opts = JwtVerifyOptions::from_value(&json!({
            "jwks_url": "https://example.org/jwks",
            "issuer": "iss-a",
            "audience": "aud-a",
            "algorithms": ["RS256", "ES256"],
        }));
        assert_eq!(opts.issuer.as_deref(), Some("iss-a"));
        assert_eq!(opts.audience.as_deref(), Some("aud-a"));
        assert_eq!(opts.algorithms, vec!["RS256", "ES256"]);
    }
}
