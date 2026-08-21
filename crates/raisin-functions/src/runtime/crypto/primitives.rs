// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure, offline crypto primitives behind `raisin.crypto.*`.
//!
//! The sibling module (`super`) verifies JWTs; this one supplies the pieces the
//! runtime was missing to *produce* trustworthy bytes in the first place:
//!
//! - **CSPRNG bytes.** Until now the only entropy a function could reach was
//!   `crypto.uuid()`. Code that needed a short unguessable code (ticket codes,
//!   nonces) was reduced to hashing `Math.random()` draws — QuickJS's PRNG is
//!   not a CSPRNG, so those codes were guessable. `random_bytes` closes that.
//! - **Digests.** `raisin.crypto.hash` is already *called* by shipped package
//!   code (dispatch-webhook computes an HMAC-ish body signature with it) but
//!   never existed, so the binding threw and signatures went out empty.
//! - **ES256 keygen + JWS signing.** `verify_jwt` could only ever check other
//!   people's tokens. Minting an offline-verifiable ticket needs the other
//!   half: a P-256 keypair we own and a compact JWS signed with it.
//!
//! Everything here is offline and side-effect free, which is why it lives in
//! `runtime::crypto` rather than `api/raisindb/crypto.rs` (policy + network).
//!
//! **Key material never reaches a log line.** No `tracing` call in this module
//! takes a private scalar, a JWK, or a signature as a field, and error strings
//! are fixed reasons that never interpolate caller input.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine as _;
use raisin_error::{Error, Result};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde_json::{json, Value};

/// Upper bound on a single `random_bytes` draw. 64 bytes is a 512-bit secret —
/// well past anything a function legitimately needs inline — and the cap keeps
/// the binding from being used as a bulk RNG pipe into the JS heap.
const MAX_RANDOM_BYTES: u32 = 64;

/// Raw byte width of a P-256 field element / scalar. `r`, `s`, `d`, `x` and `y`
/// are all exactly this wide, left-padded — JOSE requires the fixed width, so a
/// short big-endian encoding is a bug, not a shortcut.
const P256_SCALAR_LEN: usize = 32;
/// Uncompressed SEC-1 point: `0x04 || x || y`.
const P256_POINT_LEN: usize = 1 + 2 * P256_SCALAR_LEN;

/// Process-wide CSPRNG. `SystemRandom` is a handle to the OS generator; sharing
/// one avoids re-opening it on every call.
fn rng() -> &'static SystemRandom {
    use std::sync::OnceLock;
    static RNG: OnceLock<SystemRandom> = OnceLock::new();
    RNG.get_or_init(SystemRandom::new)
}

/// `n` cryptographically secure random bytes, base64url, unpadded.
///
/// Base64 rather than hex because the common caller is minting a short code and
/// wants the most entropy per character it can print.
///
/// URL-SAFE AND UNPADDED, like every other base64 in this module. The standard
/// alphabet emits `+`, `/` and `=`, which need escaping in a URL, in a QR
/// payload and in a JWS — the three places this output actually goes. Handing
/// back a string that has to be re-encoded before use is a trap for the caller.
/// The length contract, shared so the real and mock implementations cannot
/// disagree about it. Checked BEFORE any allocation.
pub fn random_bytes_check_len(n: u32) -> Result<()> {
    if n == 0 || n > MAX_RANDOM_BYTES {
        return Err(Error::Validation(format!(
            "[crypto:invalid_length] randomBytes(n): n must be 1..={MAX_RANDOM_BYTES}, got {n}"
        )));
    }
    Ok(())
}

pub fn random_bytes(n: u32) -> Result<String> {
    random_bytes_check_len(n)?;
    let mut buf = vec![0u8; n as usize];
    rng()
        .fill(&mut buf)
        .map_err(|_| Error::Backend("[crypto:rng_failed] system CSPRNG unavailable".into()))?;
    Ok(B64URL.encode(&buf))
}

/// Lowercase hex digest of `input`, `alg` defaulting to `sha256`.
///
/// Lowercase hex (not base64) because every peer that checks one of these — a
/// webhook receiver comparing an `X-Signature` header, a content address — reads
/// hex, and a case mismatch is a silent verification failure.
pub fn hash_hex(input: &str, alg: Option<&str>) -> Result<String> {
    let alg = alg.unwrap_or("sha256");
    let digest = match alg.to_ascii_lowercase().as_str() {
        "sha256" => ring::digest::digest(&ring::digest::SHA256, input.as_bytes()),
        "sha512" => ring::digest::digest(&ring::digest::SHA512, input.as_bytes()),
        _ => {
            return Err(Error::Validation(format!(
                "[crypto:unsupported_alg] hash: expected \"sha256\" or \"sha512\", got \"{alg}\""
            )))
        }
    };
    Ok(hex::encode(digest.as_ref()))
}

/// Generate a fresh signing keypair. Only `ES256` (ECDSA on P-256 with SHA-256)
/// is supported — it is what JOSE verifiers implement everywhere and what
/// `verify_jwt` can already check.
///
/// Returns `{ alg, publicJwk, privateJwk }`. The private JWK is a *return
/// value*, never a log field: the caller decides where it is stored (a secret,
/// a config node) and this module has no opinion it could leak.
pub fn generate_key_pair(alg: Option<&str>) -> Result<Value> {
    let alg = alg.unwrap_or("ES256");
    if !alg.eq_ignore_ascii_case("ES256") {
        return Err(Error::Validation(format!(
            "[crypto:unsupported_alg] generateKeyPair: only \"ES256\" is supported, got \"{alg}\""
        )));
    }

    let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, rng())
        .map_err(|_| Error::Backend("[crypto:keygen_failed] P-256 key generation failed".into()))?;
    let (d, point) = split_pkcs8_p256(doc.as_ref())?;

    // SEC-1 uncompressed: 0x04 || x || y. Anything else means ring changed its
    // output shape, which we must not paper over.
    if point.len() != P256_POINT_LEN || point[0] != 0x04 {
        return Err(Error::Backend(
            "[crypto:keygen_failed] unexpected public key encoding".into(),
        ));
    }
    let x = B64URL.encode(&point[1..=P256_SCALAR_LEN]);
    let y = B64URL.encode(&point[1 + P256_SCALAR_LEN..]);
    let kid = kid_for(&point[1..]);

    let public_jwk = json!({
        "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig",
        "kid": kid, "x": x, "y": y,
    });
    // The private JWK is the public one plus `d` — the JWK convention, and it
    // means a caller can hand the whole thing to `sign_jwt` without reassembly.
    let private_jwk = json!({
        "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig",
        "kid": kid, "x": x, "y": y, "d": B64URL.encode(d),
    });

    Ok(json!({ "alg": "ES256", "publicJwk": public_jwk, "privateJwk": private_jwk }))
}

/// Options for [`sign_jwt`], parsed from the caller's `opts` object.
#[derive(Debug, Clone, Default)]
pub struct SignJwtOptions {
    /// Requested JWS algorithm. Only `ES256` is supported.
    pub alg: Option<String>,
    /// `kid` for the JWS header. Defaults to the private JWK's own `kid`, so a
    /// verifier can select the right key out of a rotating JWKS without the
    /// caller having to remember to pass it.
    pub kid: Option<String>,
    /// When set, stamp `exp = now + expires_in_sec` over any `exp` in `claims`.
    pub expires_in_sec: Option<i64>,
}

impl SignJwtOptions {
    /// Read the options out of a caller-supplied object, tolerating `null`.
    ///
    /// PRESENT-BUT-WRONG IS AN ERROR, NOT AN ABSENCE. The obvious spelling here
    /// is `.get("expiresInSec").and_then(|v| v.as_i64())`, which silently yields
    /// `None` for anything that is not an integer — and `None` means "mint a
    /// token with no `exp`". From JavaScript that is one division away:
    ///
    ///     signJwt(c, k, { expiresInSec: ttlMs / 1000 })   // 899.5 -> no exp
    ///     signJwt(c, k, { expiresInSec: "600" })          // string -> no exp
    ///
    /// Our own verifier requires `exp`, so such a token fails locally and looks
    /// like a bug in signing. Every other JOSE verifier reads a missing `exp` as
    /// NEVER EXPIRES — a ticket signature that can never be retired. A caller who
    /// asked for a TTL and did not get one must be told.
    ///
    /// A whole-number float (`600.0`, which is what JSON gives for `600` in some
    /// encoders) is accepted, because refusing it would be pedantry rather than
    /// safety.
    pub fn from_value(opts: &Value) -> Result<Self> {
        // Only an object can carry options. A positional `signJwt(c, k, 600)`
        // arrives as a Number here and would otherwise read as "no options".
        if !opts.is_null() && !opts.is_object() {
            return Err(Error::Validation(
                "[crypto:invalid_options] signJwt: opts must be an object".to_string(),
            ));
        }

        let alg = match opts.get("alg") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => {
                return Err(Error::Validation(
                    "[crypto:invalid_options] signJwt: opts.alg must be a string".to_string(),
                ))
            }
        };

        let kid = match opts.get("kid") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => {
                return Err(Error::Validation(
                    "[crypto:invalid_options] signJwt: opts.kid must be a string".to_string(),
                ))
            }
        };

        let expires_in_sec = match opts.get("expiresInSec") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let n = v.as_i64().or_else(|| {
                    // Accept 600.0 but not 899.5 — a fractional second is far
                    // more likely to be a units mistake than an intention.
                    v.as_f64()
                        .filter(|f| f.fract() == 0.0 && f.is_finite())
                        .map(|f| f as i64)
                });
                match n {
                    Some(n) if n > 0 => Some(n),
                    _ => {
                        return Err(Error::Validation(format!(
                        "[crypto:invalid_options] signJwt: opts.expiresInSec must be a positive \
                             whole number of SECONDS, got {v}"
                    )))
                    }
                }
            }
        };

        Ok(Self {
            alg,
            kid,
            expires_in_sec,
        })
    }
}

/// Sign `claims` into a compact JWS (`header.payload.signature`) with an EC
/// private JWK.
///
/// The signature is JOSE `r||s` — 64 raw bytes for P-256, base64url without
/// padding. This is deliberate and load-bearing: ring's `*_ASN1_SIGNING`
/// variants emit a DER `SEQUENCE { r, s }` of *variable* length, which every
/// JOSE verifier rejects. We use `ECDSA_P256_SHA256_FIXED_SIGNING`, whose
/// documented output is exactly the fixed-width concatenation JWS wants.
pub fn sign_jwt(claims: &Value, private_jwk: &Value, opts: &SignJwtOptions) -> Result<String> {
    if let Some(alg) = opts.alg.as_deref() {
        if !alg.eq_ignore_ascii_case("ES256") {
            return Err(Error::Validation(format!(
                "[crypto:unsupported_alg] signJwt: only \"ES256\" is supported, got \"{alg}\""
            )));
        }
    }
    let key = ec_key_from_jwk(private_jwk)?;

    // Header. `typ: JWT` keeps the token readable by generic OIDC tooling; the
    // kid lets a verifier pick a key without trial decryption.
    let kid = opts.kid.clone().or_else(|| {
        private_jwk
            .get("kid")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    let mut header = json!({ "alg": "ES256", "typ": "JWT" });
    if let Some(kid) = kid {
        header["kid"] = json!(kid);
    }

    // Payload. `claims` must be an object: a bare scalar payload is legal JWS
    // but nothing downstream (including our own verifier) can read it.
    let mut payload = match claims {
        Value::Object(_) => claims.clone(),
        Value::Null => json!({}),
        _ => {
            return Err(Error::Validation(
                "[crypto:invalid_claims] signJwt: claims must be an object".into(),
            ))
        }
    };
    let now = chrono::Utc::now().timestamp();
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("iat").or_insert(json!(now));
        if let Some(ttl) = opts.expires_in_sec {
            if ttl <= 0 {
                return Err(Error::Validation(
                    "[crypto:invalid_expiry] signJwt: expiresInSec must be > 0".into(),
                ));
            }
            // An explicit TTL wins over any `exp` the caller also put in the
            // claims — otherwise a stale copied claim set silently outlives it.
            obj.insert("exp".into(), json!(now + ttl));
        }
    }

    let signing_input =
        format!(
            "{}.{}",
            B64URL.encode(serde_json::to_vec(&header).map_err(|_| Error::Backend(
                "[crypto:sign_failed] header not serializable".into()
            ))?),
            B64URL.encode(serde_json::to_vec(&payload).map_err(|_| Error::Validation(
                "[crypto:invalid_claims] signJwt: claims are not JSON-serializable".into()
            ))?),
        );

    let sig = key
        .sign(rng(), signing_input.as_bytes())
        .map_err(|_| Error::Backend("[crypto:sign_failed] ECDSA signing failed".into()))?;
    debug_assert_eq!(sig.as_ref().len(), 2 * P256_SCALAR_LEN);

    Ok(format!("{signing_input}.{}", B64URL.encode(sig.as_ref())))
}

/// Build a ring signing key from an EC private JWK (`crv`, `x`, `y`, `d`).
///
/// Errors are deliberately shapeless ("invalid EC private JWK"): a message that
/// said *which* component was malformed would be an oracle over key material.
fn ec_key_from_jwk(jwk: &Value) -> Result<EcdsaKeyPair> {
    let invalid = || Error::Validation("[crypto:invalid_key] invalid EC private JWK".to_string());

    if jwk.get("kty").and_then(|v| v.as_str()) != Some("EC")
        || jwk.get("crv").and_then(|v| v.as_str()) != Some("P-256")
    {
        return Err(Error::Validation(
            "[crypto:invalid_key] signJwt: privateJwk must be an EC P-256 key".into(),
        ));
    }
    let field = |name: &str| -> Result<Vec<u8>> {
        let raw = jwk.get(name).and_then(|v| v.as_str()).ok_or_else(invalid)?;
        let bytes = B64URL.decode(raw).map_err(|_| invalid())?;
        // JOSE requires fixed-width, left-padded components. Accepting a short
        // encoding here would hand ring a scalar it silently misreads.
        if bytes.len() != P256_SCALAR_LEN {
            return Err(invalid());
        }
        Ok(bytes)
    };
    let d = field("d")?;
    let x = field("x")?;
    let y = field("y")?;

    let mut point = Vec::with_capacity(P256_POINT_LEN);
    point.push(0x04);
    point.extend_from_slice(&x);
    point.extend_from_slice(&y);

    // ring re-derives the public point from `d` and rejects the pair if it does
    // not match, so a tampered JWK cannot get us to sign under a foreign key.
    EcdsaKeyPair::from_private_key_and_public_key(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &d,
        &point,
        rng(),
    )
    .map_err(|_| invalid())
}

/// A stable, non-secret key id: the first 16 bytes of SHA-256 over the raw
/// public point, base64url. Derived (not random) so the same public key always
/// gets the same `kid`, which is what makes a rotating JWKS resolvable.
fn kid_for(public_xy: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, public_xy);
    B64URL.encode(&d.as_ref()[..16])
}

/// Pull `(d, uncompressed_public_point)` out of a PKCS#8 v1 ECPrivateKey.
///
/// ring can *generate* a PKCS#8 document and *consume* one, but exposes no
/// accessor for the scalar inside, and we need it to emit a JWK. Rather than
/// rely on ring's internal template offsets (private, and free to change), this
/// walks the DER, which is fixed by RFC 5208 / RFC 5915:
///
/// ```text
/// SEQUENCE { INTEGER 0, AlgorithmIdentifier, OCTET STRING {
///     SEQUENCE { INTEGER 1, OCTET STRING d, [0] params?, [1] BIT STRING pub } } }
/// ```
fn split_pkcs8_p256(doc: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let bad = || Error::Backend("[crypto:keygen_failed] unexpected PKCS#8 encoding".to_string());

    let mut outer = Der::new(doc);
    let mut pkcs8 = Der::new(outer.tagged(0x30).ok_or_else(bad)?);
    pkcs8.tagged(0x02).ok_or_else(bad)?; // version
    pkcs8.tagged(0x30).ok_or_else(bad)?; // AlgorithmIdentifier
    let mut ec = Der::new(
        Der::new(pkcs8.tagged(0x04).ok_or_else(bad)?)
            .tagged(0x30)
            .ok_or_else(bad)?,
    );
    ec.tagged(0x02).ok_or_else(bad)?; // ECPrivateKey version
    let d = ec.tagged(0x04).ok_or_else(bad)?.to_vec();

    // Skip the optional [0] parameters, then read the [1]-tagged BIT STRING.
    let mut next = ec.peek_tag().ok_or_else(bad)?;
    if next == 0xA0 {
        ec.tagged(0xA0).ok_or_else(bad)?;
        next = ec.peek_tag().ok_or_else(bad)?;
    }
    if next != 0xA1 {
        return Err(bad());
    }
    let bit_string = Der::new(ec.tagged(0xA1).ok_or_else(bad)?)
        .tagged(0x03)
        .ok_or_else(bad)?;
    // A BIT STRING's first octet is the count of unused trailing bits; for a
    // key it is always 0, and anything else is not a point we should trust.
    match bit_string.split_first() {
        Some((0, point)) => Ok((d, point.to_vec())),
        _ => Err(bad()),
    }
}

/// Minimal forward-only DER reader — just enough for the two nested SEQUENCEs
/// above. Deliberately not a general parser: it only handles definite lengths
/// and never recurses on unknown content.
struct Der<'a> {
    buf: &'a [u8],
}

impl<'a> Der<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    /// The tag byte of the next element, without consuming it.
    fn peek_tag(&self) -> Option<u8> {
        self.buf.first().copied()
    }

    /// Consume the next element, which must carry `tag`, and return its value.
    fn tagged(&mut self, tag: u8) -> Option<&'a [u8]> {
        let (&t, rest) = self.buf.split_first()?;
        if t != tag {
            return None;
        }
        let (&first, rest) = rest.split_first()?;
        let (len, rest) = if first < 0x80 {
            (first as usize, rest)
        } else {
            // Long form: the low 7 bits count the length octets that follow.
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 || rest.len() < n {
                return None;
            }
            let (len_bytes, rest) = rest.split_at(n);
            (
                len_bytes.iter().fold(0usize, |a, &b| (a << 8) | b as usize),
                rest,
            )
        };
        if rest.len() < len {
            return None;
        }
        let (value, tail) = rest.split_at(len);
        self.buf = tail;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::crypto::{verify_with_jwks, JwtVerifyOptions};

    fn future_exp() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    /// The whole point of `hash`: shipped package code compares this digest
    /// against a peer's, so a known answer is the contract, not an example.
    #[test]
    fn hash_matches_known_sha256_answer() {
        assert_eq!(
            hash_hex("abc", None).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash_hex("abc", Some("sha256")).unwrap(),
            hash_hex("abc", None).unwrap()
        );
    }

    #[test]
    fn hash_supports_sha512_and_rejects_others() {
        let out = hash_hex("abc", Some("SHA512")).unwrap();
        assert_eq!(out.len(), 128, "sha512 is 64 bytes of hex");
        assert!(out.starts_with("ddaf35a193617aba"));
        assert!(hash_hex("abc", Some("md5")).is_err());
    }

    #[test]
    fn random_bytes_honours_length_and_range() {
        for n in [1u32, 16, 64] {
            let b64 = random_bytes(n).unwrap();
            let raw = B64URL.decode(&b64).expect("valid base64url");
            assert_eq!(raw.len(), n as usize);
        }
        assert!(random_bytes(0).is_err(), "n=0 must be refused");
        assert!(random_bytes(65).is_err(), "n>64 must be refused");
    }

    /// Not a statistical test — just proof the binding is wired to a real
    /// generator and not to a constant, which is the failure mode that would
    /// silently make every ticket code identical.
    #[test]
    fn random_bytes_are_not_constant() {
        let draws: std::collections::HashSet<String> =
            (0..32).map(|_| random_bytes(32).unwrap()).collect();
        assert_eq!(draws.len(), 32, "32 draws of 32 bytes must all differ");
    }

    #[test]
    fn generate_key_pair_shape_is_a_usable_jwk() {
        let kp = generate_key_pair(None).unwrap();
        assert_eq!(kp["alg"], "ES256");
        let pubk = &kp["publicJwk"];
        let privk = &kp["privateJwk"];
        for k in [pubk, privk] {
            assert_eq!(k["kty"], "EC");
            assert_eq!(k["crv"], "P-256");
            assert_eq!(k["alg"], "ES256");
            for f in ["x", "y"] {
                let raw = B64URL.decode(k[f].as_str().unwrap()).unwrap();
                assert_eq!(raw.len(), 32, "{f} must be a fixed-width P-256 coordinate");
            }
        }
        assert!(pubk.get("d").is_none(), "the public JWK must never carry d");
        assert_eq!(
            B64URL.decode(privk["d"].as_str().unwrap()).unwrap().len(),
            32
        );
        // The kid is derived from the public point, so both halves agree.
        assert_eq!(pubk["kid"], privk["kid"]);
        assert!(generate_key_pair(Some("RS256")).is_err());
    }

    /// Two calls must not return the same key — a cached/static keypair would
    /// make every tenant's tickets forgeable by every other tenant.
    #[test]
    fn generate_key_pair_is_fresh_each_call() {
        let a = generate_key_pair(None).unwrap();
        let b = generate_key_pair(None).unwrap();
        assert_ne!(a["privateJwk"]["d"], b["privateJwk"]["d"]);
        assert_ne!(a["publicJwk"]["kid"], b["publicJwk"]["kid"]);
    }

    /// Build a JWKS from a generated public JWK, exactly as a verifier would.
    fn jwks_of(public_jwk: &Value) -> Value {
        json!({ "keys": [public_jwk.clone()] })
    }

    /// The end-to-end contract: a token we signed must verify through the
    /// EXISTING verify path. This is what catches the DER-vs-`r||s` mistake —
    /// an ASN.1 signature parses as a JWS but fails every verifier.
    #[test]
    fn sign_jwt_round_trips_through_verify() {
        let kp = generate_key_pair(None).unwrap();
        let token = sign_jwt(
            &json!({ "sub": "ticket-42", "exp": future_exp() }),
            &kp["privateJwk"],
            &SignJwtOptions::default(),
        )
        .unwrap();

        let out = verify_with_jwks(
            &token,
            &jwks_of(&kp["publicJwk"]),
            &JwtVerifyOptions::default(),
        );
        assert_eq!(out["valid"], json!(true), "out={out}");
        assert_eq!(out["claims"]["sub"], json!("ticket-42"));
        assert!(out["claims"]["iat"].is_i64(), "iat is stamped when absent");
    }

    /// The signature must be exactly 64 raw bytes of unpadded base64url, and
    /// the header must advertise ES256 with the key's kid.
    #[test]
    fn sign_jwt_emits_jose_fixed_width_signature() {
        let kp = generate_key_pair(None).unwrap();
        let token = sign_jwt(
            &json!({ "sub": "a" }),
            &kp["privateJwk"],
            &SignJwtOptions::default(),
        )
        .unwrap();

        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "compact JWS has three parts");
        assert!(!token.contains('='), "base64url in a JWS is unpadded");
        assert!(
            !token.contains('+') && !token.contains('/'),
            "url-safe alphabet only"
        );

        let header: Value = serde_json::from_slice(&B64URL.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["typ"], "JWT");
        assert_eq!(header["kid"], kp["publicJwk"]["kid"]);

        let sig = B64URL.decode(parts[2]).unwrap();
        assert_eq!(sig.len(), 64, "ES256 is r||s, 32+32 raw bytes — not DER");
    }

    #[test]
    fn sign_jwt_expires_in_sec_overrides_claims_exp() {
        let kp = generate_key_pair(None).unwrap();
        let opts = SignJwtOptions {
            expires_in_sec: Some(60),
            ..Default::default()
        };
        // A stale `exp` copied into the claims must not survive an explicit TTL.
        let token = sign_jwt(
            &json!({ "sub": "a", "exp": 1_i64 }),
            &kp["privateJwk"],
            &opts,
        )
        .unwrap();
        let payload: Value =
            serde_json::from_slice(&B64URL.decode(token.split('.').nth(1).unwrap()).unwrap())
                .unwrap();
        let exp = payload["exp"].as_i64().unwrap();
        assert!(
            exp > chrono::Utc::now().timestamp(),
            "exp={exp} must be in the future"
        );

        let out = verify_with_jwks(
            &token,
            &jwks_of(&kp["publicJwk"]),
            &JwtVerifyOptions::default(),
        );
        assert_eq!(out["valid"], json!(true), "out={out}");
    }

    /// An expired token must be rejected by the existing verifier — proving the
    /// TTL we stamp is the one `exp` the verifier reads.
    #[test]
    fn sign_jwt_expired_token_fails_verification() {
        let kp = generate_key_pair(None).unwrap();
        let past = chrono::Utc::now().timestamp() - 3600;
        let token = sign_jwt(
            &json!({ "sub": "a", "exp": past }),
            &kp["privateJwk"],
            &SignJwtOptions::default(),
        )
        .unwrap();
        let out = verify_with_jwks(
            &token,
            &jwks_of(&kp["publicJwk"]),
            &JwtVerifyOptions::default(),
        );
        assert_eq!(out["valid"], json!(false));
        assert_eq!(out["error"], json!("token expired"));
    }

    /// A token signed by one key must not verify under another's JWKS.
    #[test]
    fn sign_jwt_does_not_verify_under_a_foreign_key() {
        let mine = generate_key_pair(None).unwrap();
        let theirs = generate_key_pair(None).unwrap();
        let token = sign_jwt(
            &json!({ "sub": "a", "exp": future_exp() }),
            &mine["privateJwk"],
            &SignJwtOptions {
                // Force the foreign kid so key selection succeeds and the
                // SIGNATURE is what fails — otherwise this would only prove
                // that kid lookup works.
                kid: theirs["publicJwk"]["kid"].as_str().map(str::to_string),
                ..Default::default()
            },
        )
        .unwrap();
        let out = verify_with_jwks(
            &token,
            &jwks_of(&theirs["publicJwk"]),
            &JwtVerifyOptions::default(),
        );
        assert_eq!(out["valid"], json!(false), "out={out}");
    }

    #[test]
    fn sign_jwt_rejects_bad_keys_and_algs() {
        let kp = generate_key_pair(None).unwrap();
        let good = kp["privateJwk"].clone();

        // Unsupported algorithm.
        let opts = SignJwtOptions {
            alg: Some("RS256".into()),
            ..Default::default()
        };
        assert!(sign_jwt(&json!({}), &good, &opts).is_err());

        // Missing `d` -> not a private key.
        let mut no_d = good.clone();
        no_d.as_object_mut().unwrap().remove("d");
        assert!(sign_jwt(&json!({}), &no_d, &SignJwtOptions::default()).is_err());

        // Wrong curve.
        let mut wrong_crv = good.clone();
        wrong_crv["crv"] = json!("P-384");
        assert!(sign_jwt(&json!({}), &wrong_crv, &SignJwtOptions::default()).is_err());

        // `d` that does not match `x`/`y` — ring re-derives the point, so this
        // must be refused rather than signed under a mismatched identity.
        let other = generate_key_pair(None).unwrap();
        let mut mismatched = good.clone();
        mismatched["d"] = other["privateJwk"]["d"].clone();
        assert!(sign_jwt(&json!({}), &mismatched, &SignJwtOptions::default()).is_err());

        // Short (non-left-padded) component.
        let mut short = good.clone();
        short["x"] = json!(B64URL.encode([1u8; 31]));
        assert!(sign_jwt(&json!({}), &short, &SignJwtOptions::default()).is_err());

        // Non-object claims.
        assert!(sign_jwt(&json!("nope"), &good, &SignJwtOptions::default()).is_err());
        // Non-positive TTL.
        let opts = SignJwtOptions {
            expires_in_sec: Some(0),
            ..Default::default()
        };
        assert!(sign_jwt(&json!({}), &good, &opts).is_err());
    }

    /// No error message may echo key material — an error string is a common
    /// path into a log line, and that is exactly how private scalars leak.
    #[test]
    fn key_errors_never_echo_the_secret() {
        let kp = generate_key_pair(None).unwrap();
        let d = kp["privateJwk"]["d"].as_str().unwrap().to_string();
        let mut broken = kp["privateJwk"].clone();
        broken["y"] = json!(B64URL.encode([9u8; 32]));
        let err = sign_jwt(&json!({}), &broken, &SignJwtOptions::default()).unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains(&d),
            "error message leaked the private scalar: {msg}"
        );
        assert!(msg.contains("invalid_key"));
    }

    #[test]
    fn options_parse_from_value() {
        let o = SignJwtOptions::from_value(&json!({
            "alg": "ES256", "kid": "k1", "expiresInSec": 900
        }))
        .expect("well-formed options");
        assert_eq!(o.alg.as_deref(), Some("ES256"));
        assert_eq!(o.kid.as_deref(), Some("k1"));
        assert_eq!(o.expires_in_sec, Some(900));

        let empty = SignJwtOptions::from_value(&Value::Null).expect("null is no options");
        assert!(empty.alg.is_none() && empty.kid.is_none() && empty.expires_in_sec.is_none());
    }

    /// A TTL that is present but unusable must be REFUSED, never dropped.
    ///
    /// Dropping it mints a token with no `exp`. Our own verifier requires one so
    /// it fails here and looks like a signing bug; every other JOSE verifier
    /// reads a missing `exp` as never-expires, which on a ticket is a signature
    /// that can never be retired. Each case below is one keystroke away in JS.
    #[test]
    fn options_reject_a_ttl_that_would_silently_vanish() {
        for bad in [
            json!({ "expiresInSec": 899.5 }), // ttlMs / 1000, not exact
            json!({ "expiresInSec": "600" }), // a string from a form or env
            json!({ "expiresInSec": 0 }),     // "no expiry", spelled as a number
            json!({ "expiresInSec": -60 }),   // already expired
            json!({ "expiresInSec": true }),
            json!({ "expiresInSec": [600] }),
        ] {
            let err = SignJwtOptions::from_value(&bad)
                .expect_err(&format!("must reject {bad}"))
                .to_string();
            assert!(
                err.contains("expiresInSec"),
                "error should name the option: {err}"
            );
        }

        // 600.0 is what some JSON encoders produce for 600. Refusing it would be
        // pedantry, not safety.
        let ok = SignJwtOptions::from_value(&json!({ "expiresInSec": 600.0 }))
            .expect("a whole-number float is a whole number");
        assert_eq!(ok.expires_in_sec, Some(600));
    }

    /// `signJwt(claims, key, 600)` — a TTL passed positionally — used to parse
    /// as "no options at all" and mint a token with no expiry.
    #[test]
    fn options_reject_a_non_object() {
        assert!(SignJwtOptions::from_value(&json!(600)).is_err());
        assert!(SignJwtOptions::from_value(&json!("ES256")).is_err());
        assert!(SignJwtOptions::from_value(&json!({ "alg": 123 })).is_err());
        assert!(SignJwtOptions::from_value(&json!({ "kid": ["k1"] })).is_err());
    }

    #[test]
    fn random_bytes_rejects_a_length_that_would_over_allocate() {
        // ArgParser::u32 casts with `as u32`, so randomBytes(-1) arrives here as
        // u32::MAX. The check must run before any allocation.
        assert!(random_bytes(u32::MAX).is_err());
        assert!(random_bytes_check_len(u32::MAX).is_err());
        assert!(random_bytes_check_len(0).is_err());
        assert!(random_bytes_check_len(32).is_ok());
    }

    /// randomBytes output goes into URLs, QR payloads and JWS segments, so it
    /// must not carry `+`, `/` or `=`.
    #[test]
    fn random_bytes_is_url_safe_and_unpadded() {
        for n in 1..=MAX_RANDOM_BYTES {
            let out = random_bytes(n).expect("in range");
            assert!(
                !out.contains('+') && !out.contains('/') && !out.contains('='),
                "randomBytes({n}) was not base64url-unpadded: {out}"
            );
        }
    }
}
