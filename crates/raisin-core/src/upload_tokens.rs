// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Signed, one-time upload tokens.
//!
//! A token lets a device WITHOUT a user (a phone that scanned a QR code) upload
//! into ONE destination that a signed-in user authorized when they minted it.
//!
//! # What the token is, and is not
//!
//! The minter's `create` permission on the destination is checked at MINT time
//! by the HTTP layer; the token carries no user identity and no permissions. It
//! names a destination (an exact path, or a folder in which the server picks
//! the file name), a size cap, a mime allow-list, an expiry and a use count,
//! and it is only ever honoured by the upload endpoints. Nothing here grants a
//! read, and no other route parses it.
//!
//! # Signing
//!
//! HMAC-SHA256 over the base64url payload. The key is derived PER TENANT from
//! the engine signing secret (`HMAC(secret, domain || tenant)`), so a token
//! minted for tenant A fails the signature on tenant B before any claim is
//! parsed. A distinct domain separator keeps a string that verifies as an
//! asset grant or asset signature from ever verifying as an upload token.
//! Verification takes a LIST of secrets (current first) so a rotated secret
//! still verifies tokens minted under the previous one within their lifetime.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

const TOKEN_PREFIX: &str = "rut1";
const KEY_DOMAIN: &[u8] = b"raisin:upload-token:key:v1\0";
const SIGN_DOMAIN: &[u8] = b"raisin:upload-token:sig:v1\0";

/// Longest lifetime a token may be minted for.
pub const MAX_UPLOAD_TOKEN_TTL_SECS: u64 = 3600;
/// Lifetime when the minter names none.
pub const DEFAULT_UPLOAD_TOKEN_TTL_SECS: u64 = 600;
/// Most uses one token may carry (folder mode only; an exact path is one use).
pub const MAX_UPLOAD_TOKEN_USES: u32 = 20;

/// The signed claim. Every field is part of the signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadTokenClaims {
    pub tenant_id: String,
    pub repo: String,
    pub branch: String,
    pub workspace: String,
    /// Exact destination path. Exactly one of `path` and `folder` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Destination folder; the server chooses a safe file name per use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// File extension for folder mode (`jpg`), already sanitized. May be empty.
    #[serde(default)]
    pub ext: String,
    pub node_type: String,
    /// Largest accepted file, in bytes.
    pub max_bytes: u64,
    /// Accepted content types; `image/*` style wildcards allowed. Empty = any.
    #[serde(default)]
    pub mimes: Vec<String>,
    /// Unix seconds after which no NEW upload session may be opened.
    pub exp: u64,
    /// Unique token id.
    pub jti: String,
    /// How many upload sessions the token may open.
    pub uses: u32,
}

/// Why a token was refused. The HTTP layer maps every one to the same 401 body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadTokenError {
    Malformed,
    BadSignature,
    WrongTenant,
}

impl UploadTokenClaims {
    /// The destination for use number `seq` (0-based).
    pub fn destination(&self, seq: u32) -> String {
        if let Some(path) = &self.path {
            return path.clone();
        }
        let folder = self.folder.as_deref().unwrap_or("/").trim_end_matches('/');
        let short: String = self.jti.chars().take(8).collect();
        let ext = if self.ext.is_empty() {
            String::new()
        } else {
            format!(".{}", self.ext)
        };
        format!("{folder}/up-{short}-{seq}{ext}")
    }

    /// Does `content_type` pass the allow-list?
    pub fn mime_allowed(&self, content_type: Option<&str>) -> bool {
        if self.mimes.is_empty() {
            return true;
        }
        let Some(ct) = content_type else {
            return false;
        };
        // Parameters (`; charset=...`) are not part of the type.
        let ct = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        self.mimes.iter().any(|m| {
            let m = m.trim().to_ascii_lowercase();
            match m.strip_suffix("/*") {
                Some(major) => ct.split('/').next() == Some(major) && ct.contains('/'),
                None => m == ct,
            }
        })
    }
}

/// The per-tenant key. Never leaves this module.
fn derive_key(secret: &[u8], tenant_id: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(KEY_DOMAIN);
    mac.update(tenant_id.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn sign(key: &[u8], payload_b64: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(SIGN_DOMAIN);
    mac.update(payload_b64.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Mint a token for `claims.tenant_id` under `secret`.
pub fn mint_upload_token(secret: &[u8], claims: &UploadTokenClaims) -> String {
    let payload =
        serde_json::to_vec(claims).expect("UploadTokenClaims is a plain struct and serializes");
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload);
    let sig = sign(&derive_key(secret, &claims.tenant_id), &payload_b64);
    format!(
        "{TOKEN_PREFIX}.{payload_b64}.{}",
        URL_SAFE_NO_PAD.encode(sig)
    )
}

/// Verify signature and tenant. Does NOT check expiry or use count: expiry only
/// gates opening a session, which the caller decides.
///
/// `secrets` is the current signing secret followed by any previous one still
/// accepted. The signature is checked against the key derived for the REQUEST's
/// tenant, over the bytes as they arrived, before the payload is parsed.
pub fn verify_upload_token(
    secrets: &[Vec<u8>],
    request_tenant: &str,
    token: &str,
) -> Result<UploadTokenClaims, UploadTokenError> {
    let mut parts = token.split('.');
    let (Some(version), Some(payload_b64), Some(sig_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(UploadTokenError::Malformed);
    };
    if version != TOKEN_PREFIX {
        return Err(UploadTokenError::Malformed);
    }
    let sig = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| UploadTokenError::Malformed)?;

    // Check every secret (no early exit on the first match's position) so the
    // time taken does not say which one signed.
    let mut ok = false;
    for secret in secrets {
        let expected = sign(&derive_key(secret, request_tenant), payload_b64);
        ok |= bool::from(expected.as_slice().ct_eq(sig.as_slice()));
    }
    if !ok {
        return Err(UploadTokenError::BadSignature);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| UploadTokenError::Malformed)?;
    let claims: UploadTokenClaims =
        serde_json::from_slice(&payload).map_err(|_| UploadTokenError::Malformed)?;
    // Defence in depth: the key already bound the tenant.
    if claims.tenant_id != request_tenant {
        return Err(UploadTokenError::WrongTenant);
    }
    if claims.path.is_some() == claims.folder.is_some() || claims.jti.is_empty() {
        return Err(UploadTokenError::Malformed);
    }
    Ok(claims)
}

/// Seconds since the epoch.
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> UploadTokenClaims {
        UploadTokenClaims {
            tenant_id: "default".into(),
            repo: "studio".into(),
            branch: "main".into(),
            workspace: "assets".into(),
            path: Some("/inbound/a.jpg".into()),
            folder: None,
            ext: String::new(),
            node_type: "raisin:Asset".into(),
            max_bytes: 1000,
            mimes: vec!["image/*".into()],
            exp: now_unix_secs() + 600,
            jti: "abcdefghijkl".into(),
            uses: 1,
        }
    }

    fn secrets(s: &str) -> Vec<Vec<u8>> {
        vec![s.as_bytes().to_vec()]
    }

    #[test]
    fn round_trip() {
        let c = claims();
        let t = mint_upload_token(b"s1", &c);
        assert_eq!(
            verify_upload_token(&secrets("s1"), "default", &t).unwrap(),
            c
        );
    }

    #[test]
    fn tampered_payload_or_signature_is_refused() {
        let t = mint_upload_token(b"s1", &claims());
        let parts: Vec<&str> = t.split('.').collect();
        let mut forged = claims();
        forged.path = Some("/elsewhere/x.jpg".into());
        forged.max_bytes = u64::MAX;
        let forged_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
        let swapped = format!("rut1.{forged_b64}.{}", parts[2]);
        assert_eq!(
            verify_upload_token(&secrets("s1"), "default", &swapped),
            Err(UploadTokenError::BadSignature)
        );
        let bad_sig = format!("rut1.{}.{}", parts[1], URL_SAFE_NO_PAD.encode([0u8; 32]));
        assert_eq!(
            verify_upload_token(&secrets("s1"), "default", &bad_sig),
            Err(UploadTokenError::BadSignature)
        );
        for junk in ["", "rut1", "rut1.a", "rut1.a.b.c", "rut2.a.b", "x.y.z"] {
            assert!(verify_upload_token(&secrets("s1"), "default", junk).is_err());
        }
    }

    #[test]
    fn other_tenant_or_other_secret_is_refused() {
        let t = mint_upload_token(b"s1", &claims());
        assert_eq!(
            verify_upload_token(&secrets("s1"), "other", &t),
            Err(UploadTokenError::BadSignature)
        );
        assert_eq!(
            verify_upload_token(&secrets("s2"), "default", &t),
            Err(UploadTokenError::BadSignature)
        );
    }

    #[test]
    fn rotated_secret_still_verifies_old_tokens() {
        let t = mint_upload_token(b"old", &claims());
        let both = vec![b"new".to_vec(), b"old".to_vec()];
        assert!(verify_upload_token(&both, "default", &t).is_ok());
        let only_new = vec![b"new".to_vec()];
        assert!(verify_upload_token(&only_new, "default", &t).is_err());
    }

    #[test]
    fn an_asset_grant_is_not_an_upload_token() {
        let grant = crate::mint_asset_grant(
            b"s1",
            &crate::AssetGrant {
                tenant_id: "default".into(),
                repo: "studio".into(),
                branch: "main".into(),
                workspace: "assets".into(),
                prefix: "/".into(),
                subject: "u".into(),
                email: None,
                home: None,
                expires: now_unix_secs() + 60,
            },
        );
        assert!(verify_upload_token(&secrets("s1"), "default", &grant).is_err());
    }

    #[test]
    fn claims_need_exactly_one_destination() {
        let mut c = claims();
        c.folder = Some("/inbound".into());
        let t = mint_upload_token(b"s1", &c);
        assert_eq!(
            verify_upload_token(&secrets("s1"), "default", &t),
            Err(UploadTokenError::Malformed)
        );
        c.path = None;
        c.folder = None;
        let t = mint_upload_token(b"s1", &c);
        assert!(verify_upload_token(&secrets("s1"), "default", &t).is_err());
    }

    #[test]
    fn destinations() {
        let c = claims();
        assert_eq!(c.destination(0), "/inbound/a.jpg");
        let mut f = claims();
        f.path = None;
        f.folder = Some("/inbound/phone/".into());
        f.ext = "jpg".into();
        assert_eq!(f.destination(2), "/inbound/phone/up-abcdefgh-2.jpg");
        f.ext = String::new();
        assert_eq!(f.destination(0), "/inbound/phone/up-abcdefgh-0");
    }

    #[test]
    fn mime_allow_list() {
        let c = claims();
        assert!(c.mime_allowed(Some("image/jpeg")));
        assert!(c.mime_allowed(Some("IMAGE/PNG; q=1")));
        assert!(!c.mime_allowed(Some("application/pdf")));
        assert!(!c.mime_allowed(Some("image")));
        assert!(!c.mime_allowed(None));
        let mut any = claims();
        any.mimes.clear();
        assert!(any.mime_allowed(None));
        let mut exact = claims();
        exact.mimes = vec!["application/pdf".into()];
        assert!(exact.mime_allowed(Some("application/pdf")));
        assert!(!exact.mime_allowed(Some("application/pdf2")));
    }
}
