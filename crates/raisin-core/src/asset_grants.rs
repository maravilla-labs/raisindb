// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Scoped asset access grants, and the ONE authorization entry point that
//! accepts them alongside per-asset signatures.
//!
//! # Why a second form exists
//!
//! A signed asset URL ([`crate::asset_urls`]) authorizes exactly ONE object. For
//! a process that fetches one file that is precisely right: a narrow, disposable
//! capability that carries no ambient authority. For a page that shows sixty
//! thumbnails it is sixty signatures on sixty independent clocks, so a tab left
//! open long enough starts serving broken images.
//!
//! A GRANT signs a SCOPE instead — `(tenant, repo, branch, workspace, path
//! prefix, subject, expiry)` — and every asset under that prefix validates
//! against the same token. One round trip per page load, one clock.
//!
//! # What keeps widening the scope honest
//!
//! A grant is NOT a bigger signature. A signature IS the authority: the serve
//! path reads the node without consulting row-level security, because the
//! minter already did. A grant deliberately is not that. It names a SUBJECT, and
//! the read it authorizes must be performed as that subject, under that
//! subject's row-level security, at the moment of the read. So a grant cannot
//! return anything a direct read by the same subject would not — not by
//! assumption, but because it is the same read.
//!
//! Three consequences worth stating, because they are the whole security
//! argument:
//!
//! 1. **A grant cannot exceed its subject.** It confers no authority of its own;
//!    it only says WHICH subject and WHICH subtree. Minting one for a prefix
//!    holding nodes the subject may not read is harmless — those nodes stay
//!    unreadable.
//! 2. **Revocation is the permission system's, not the token's.** Permissions
//!    are resolved on the read path, so withdrawing a subject's access stops the
//!    grant working as soon as the resolver stops returning that access. The
//!    expiry is the outer bound, not the mechanism.
//! 3. **Scope is matched on path SEGMENTS.** A grant for `/photos` must not
//!    cover `/photos-private`. String prefixes get this wrong; see
//!    [`prefix_covers`].
//!
//! # What this does NOT replace
//!
//! Per-asset signing, for anything server-to-server. Handing a subtree-wide
//! token to an out-of-process media service would be strictly worse than handing
//! it the one URL it needs. The rule is by AUDIENCE: interactive session →
//! grant; one machine fetching one object → signature.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Token version marker, and the first component of every grant token.
///
/// Present so a future payload shape can be introduced without a verifier
/// having to guess which one it is holding: an unknown marker is rejected
/// outright rather than parsed hopefully.
const GRANT_TOKEN_PREFIX: &str = "rag1";

/// Domain separator mixed into the HMAC input.
///
/// The asset-URL signature and a grant are computed with the SAME secret. Domain
/// separation is what stops a string that verifies as one from ever verifying as
/// the other.
const GRANT_DOMAIN: &[u8] = b"raisin:asset-grant:v1\0";

/// The scope a grant authorizes, exactly as it is signed.
///
/// Every field is part of the signed payload, so none of them can be changed by
/// a client. The identity fields (`subject`, `email`, `home`) are copied from
/// the MINTING principal's own authenticated context — never from the mint
/// request — which is what makes a grant unable to name someone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetGrant {
    /// Tenant the grant is valid in. A grant minted in one tenant is refused in
    /// another even though the path shapes are identical.
    pub tenant_id: String,
    /// Repository the grant is valid in.
    pub repo: String,
    /// Branch the grant is valid on. A working-view grant does not open the
    /// published branch, and vice versa.
    pub branch: String,
    /// Workspace the grant is valid in.
    pub workspace: String,
    /// Node path prefix, normalized and matched SEGMENT-WISE — see
    /// [`prefix_covers`]. `/` covers the whole workspace.
    pub prefix: String,
    /// The subject the read is performed AS: the identity id the minting
    /// principal authenticated with.
    pub subject: String,
    /// The subject's email, as their session asserted it. Carried because
    /// row-level security conditions may reference it, and a read that silently
    /// lost it would evaluate differently from the same read on the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// The subject's home path, carried for the same reason as `email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Unix seconds at which the grant stops being accepted.
    pub expires: u64,
}

/// Why a presented credential was refused.
///
/// Distinguished for the log line and for the response code, never for the
/// response body's detail: a caller learns only that it must re-authorize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetAuthError {
    /// Neither a signature nor a grant was presented.
    MissingCredential,
    /// A grant token that is not a grant token: wrong version marker, wrong
    /// shape, undecodable payload.
    MalformedGrant,
    /// Signature or grant did not verify against the secret.
    BadSignature,
    /// Verified, but past its expiry.
    Expired,
    /// Verified and live, but does not cover the asset being asked for —
    /// another tenant, repo, branch, workspace, or a path outside the prefix.
    OutOfScope,
}

impl AssetAuthError {
    /// A stable machine-readable code for the HTTP error body.
    pub fn code(&self) -> &'static str {
        match self {
            AssetAuthError::MissingCredential => "MISSING_CREDENTIAL",
            AssetAuthError::MalformedGrant => "INVALID_GRANT",
            AssetAuthError::BadSignature => "INVALID_SIGNATURE",
            AssetAuthError::Expired => "CREDENTIAL_EXPIRED",
            AssetAuthError::OutOfScope => "GRANT_OUT_OF_SCOPE",
        }
    }

    /// `true` when re-authorizing (minting a fresh grant) could plausibly fix
    /// it. The client contract is "renew on 401, once"; this is what it renews
    /// on.
    pub fn is_renewable(&self) -> bool {
        matches!(self, AssetAuthError::Expired)
    }
}

impl std::fmt::Display for AssetAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            AssetAuthError::MissingCredential => "No signature or grant presented",
            AssetAuthError::MalformedGrant => "Grant is not a readable grant token",
            AssetAuthError::BadSignature => "Invalid signature",
            AssetAuthError::Expired => "Credential has expired",
            AssetAuthError::OutOfScope => "Grant does not cover this asset",
        };
        f.write_str(msg)
    }
}

/// The asset a credential is being checked against.
#[derive(Debug, Clone, Copy)]
pub struct AssetReadScope<'a> {
    pub tenant_id: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
    /// The node path, with or without a leading slash.
    pub node_path: &'a str,
    /// The property being read (`file`, `thumbnail`, …).
    pub property: &'a str,
    /// `display` or `download`.
    pub command: &'a str,
}

/// What the caller presented.
#[derive(Debug, Clone, Copy)]
pub enum AssetCredential<'a> {
    /// The historical per-asset form: `?sig=…&exp=…`.
    Signature { sig: &'a str, expires: u64 },
    /// The scoped form: `?grant=…`.
    Grant { token: &'a str },
}

impl<'a> AssetCredential<'a> {
    /// Pick the credential a request carries, preferring an explicit grant.
    ///
    /// A request may not present both meaningfully, and a client that sends
    /// both gets the grant checked: it is the narrower authority of the two,
    /// since it is evaluated against the subject's own row-level security.
    pub fn from_query(
        sig: Option<&'a str>,
        exp: Option<u64>,
        grant: Option<&'a str>,
    ) -> Result<Self, AssetAuthError> {
        if let Some(token) = grant.filter(|t| !t.is_empty()) {
            return Ok(AssetCredential::Grant { token });
        }
        match sig.filter(|s| !s.is_empty()) {
            Some(sig) => Ok(AssetCredential::Signature {
                sig,
                expires: exp.unwrap_or(0),
            }),
            None => Err(AssetAuthError::MissingCredential),
        }
    }
}

/// The outcome of a successful check, and the instruction that comes with it.
#[derive(Debug, Clone)]
pub enum AssetAuthorization {
    /// A per-asset signature verified. The signature IS the authority: the
    /// minter checked access when it signed, and the read proceeds as it always
    /// has.
    Signature,
    /// A grant verified and covers this asset. It is NOT authority on its own —
    /// the caller MUST perform the read as [`AssetGrant::subject`], under that
    /// subject's row-level security. Reading it any other way would turn a
    /// scope assertion into a privilege.
    Grant(Box<AssetGrant>),
}

/// The single entry point both credential forms go through.
///
/// One function rather than two branches in the serve handler, because a
/// verifier that forks drifts, and a drifted verifier is a URL that answers 401
/// with nothing to say why — the signature is a hash, so there is no diff to
/// read.
pub fn authorize_asset_read(
    secret: &[u8],
    scope: AssetReadScope<'_>,
    credential: AssetCredential<'_>,
) -> Result<AssetAuthorization, AssetAuthError> {
    match credential {
        AssetCredential::Signature { sig, expires } => {
            if sig.is_empty() {
                return Err(AssetAuthError::MissingCredential);
            }
            if expires < now_unix() {
                return Err(AssetAuthError::Expired);
            }
            let signed_path = crate::asset_urls::signed_asset_path(
                scope.repo,
                scope.branch,
                scope.workspace,
                scope.node_path,
                scope.property,
            );
            let ok = crate::verify_asset_signature(
                secret,
                scope.tenant_id,
                &signed_path,
                scope.command,
                crate::asset_urls::signature_property(scope.property),
                expires,
                sig,
            );
            if ok {
                Ok(AssetAuthorization::Signature)
            } else {
                Err(AssetAuthError::BadSignature)
            }
        }
        AssetCredential::Grant { token } => {
            let grant = decode_grant(secret, token)?;
            if grant.expires < now_unix() {
                return Err(AssetAuthError::Expired);
            }
            if !grant.covers(&scope) {
                return Err(AssetAuthError::OutOfScope);
            }
            Ok(AssetAuthorization::Grant(Box::new(grant)))
        }
    }
}

impl AssetGrant {
    /// `true` when this grant's scope contains the asset being read.
    ///
    /// Expiry is deliberately NOT checked here — [`authorize_asset_read`] owns
    /// the order of checks so no caller can accidentally test coverage and
    /// forget the clock.
    pub fn covers(&self, scope: &AssetReadScope<'_>) -> bool {
        self.tenant_id == scope.tenant_id
            && self.repo == scope.repo
            && self.branch == scope.branch
            && self.workspace == scope.workspace
            && prefix_covers(&self.prefix, scope.node_path)
    }
}

/// Mint a grant token.
///
/// The returned string is opaque to the client and goes on an asset URL as
/// `?grant=…`. It is URL-safe by construction.
pub fn mint_asset_grant(secret: &[u8], grant: &AssetGrant) -> String {
    let payload =
        serde_json::to_vec(grant).expect("AssetGrant is a plain struct and always serializes");
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_payload(secret, &payload_b64);
    format!("{}.{}.{}", GRANT_TOKEN_PREFIX, payload_b64, signature)
}

/// Verify a grant token's signature and return the scope it carries.
///
/// Does NOT check expiry or coverage — those belong to
/// [`authorize_asset_read`], which owns the whole decision. Exposed for tests
/// and for tooling that needs to read a token back.
pub fn decode_grant(secret: &[u8], token: &str) -> Result<AssetGrant, AssetAuthError> {
    let mut parts = token.split('.');
    let (Some(version), Some(payload_b64), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(AssetAuthError::MalformedGrant);
    };

    if version != GRANT_TOKEN_PREFIX {
        return Err(AssetAuthError::MalformedGrant);
    }

    // Signature FIRST, over the encoded payload exactly as it arrived. Checking
    // the bytes on the wire rather than a re-encoding of the parsed value means
    // no serializer detail — key order, whitespace, number formatting — can ever
    // make a valid token look invalid, and no parser is run on unauthenticated
    // input before the secret has had its say.
    let expected = sign_payload(secret, payload_b64);
    let matches: bool = expected.as_bytes().ct_eq(signature.as_bytes()).into();
    if !matches {
        return Err(AssetAuthError::BadSignature);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| AssetAuthError::MalformedGrant)?;
    let grant: AssetGrant =
        serde_json::from_slice(&payload).map_err(|_| AssetAuthError::MalformedGrant)?;

    // A prefix that cannot be normalized is a prefix that cannot be matched
    // safely. Refuse the token rather than fall back to a string comparison.
    if normalize_path(&grant.prefix).is_none() || grant.subject.is_empty() {
        return Err(AssetAuthError::MalformedGrant);
    }

    Ok(grant)
}

/// Does `prefix` contain `path`, matching on path SEGMENTS?
///
/// This is the one place where a subtly wrong line becomes a data leak. A string
/// prefix test says `/photos` contains `/photos-private`, because the characters
/// line up; a segment test says it does not, because `photos-private` is a
/// different name. Everything below exists to make the second answer the only
/// one reachable.
///
/// Both sides are normalized first, and a path that cannot be normalized — an
/// empty segment, a `.` or a `..` — is refused rather than guessed at. `/` is
/// the whole workspace and covers everything.
pub fn prefix_covers(prefix: &str, path: &str) -> bool {
    let (Some(prefix), Some(path)) = (normalize_path(prefix), normalize_path(path)) else {
        return false;
    };

    if prefix == "/" {
        return true;
    }
    if path == prefix {
        return true;
    }

    // The trailing slash is what makes this a SEGMENT test: `/photos/` is a
    // prefix of `/photos/a.jpg` and is not a prefix of `/photos-private/a.jpg`.
    path.starts_with(&format!("{}/", prefix))
}

/// A path in its one canonical spelling, or `None` when it has no safe one.
///
/// Rejects `.` and `..` segments outright. Traversal is not something to
/// resolve here: a prefix or a path that contains it is malformed input from a
/// surface that should never produce it, and resolving it quietly would mean
/// two spellings of one path — precisely the condition a segment test exists to
/// rule out.
pub fn normalize_path(path: &str) -> Option<String> {
    let mut out = String::from("/");
    let mut first = true;
    for segment in path.split('/') {
        if segment.is_empty() {
            // Leading, trailing and doubled slashes collapse; they carry no
            // meaning in a node path.
            continue;
        }
        if segment == "." || segment == ".." {
            return None;
        }
        if !first {
            out.push('/');
        }
        out.push_str(segment);
        first = false;
    }
    Some(out)
}

/// The largest lifetime a grant may be minted for.
///
/// A grant is bounded by BOTH this and the subject's live permissions, which are
/// resolved on the read path. The cap is the outer bound on a stolen token, not
/// the revocation mechanism.
pub const MAX_GRANT_LIFETIME_SECS: u64 = 3600;

/// The lifetime a grant gets when the caller names none.
///
/// Long enough that an ordinary page load does not renew mid-view, short enough
/// that a leaked token is stale before it travels. The client renews on 401.
pub const DEFAULT_GRANT_LIFETIME_SECS: u64 = 900;

/// Clamp a requested lifetime into what is allowed.
pub fn clamp_grant_lifetime(requested: u64) -> u64 {
    requested.clamp(1, MAX_GRANT_LIFETIME_SECS)
}

fn sign_payload(secret: &[u8], payload_b64: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(GRANT_DOMAIN);
    mac.update(payload_b64.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-key-32-bytes-long!!!";
    const OTHER_SECRET: &[u8] = b"a-different-secret-32-bytes!!!!!";

    fn grant() -> AssetGrant {
        AssetGrant {
            tenant_id: "tenant-a".into(),
            repo: "media".into(),
            branch: "main".into(),
            workspace: "assets".into(),
            prefix: "/photos".into(),
            subject: "user-1".into(),
            email: Some("user@example.test".into()),
            home: None,
            expires: u64::MAX,
        }
    }

    fn scope<'a>(path: &'a str) -> AssetReadScope<'a> {
        AssetReadScope {
            tenant_id: "tenant-a",
            repo: "media",
            branch: "main",
            workspace: "assets",
            node_path: path,
            property: "file",
            command: "display",
        }
    }

    #[test]
    fn a_minted_grant_round_trips() {
        let token = mint_asset_grant(SECRET, &grant());
        assert_eq!(decode_grant(SECRET, &token).unwrap(), grant());
    }

    #[test]
    fn a_grant_authorizes_every_asset_under_its_prefix() {
        let token = mint_asset_grant(SECRET, &grant());
        for path in ["/photos/a.jpg", "/photos/2024/b.png", "/photos"] {
            let decision = authorize_asset_read(
                SECRET,
                scope(path),
                AssetCredential::Grant { token: &token },
            );
            assert!(
                matches!(decision, Ok(AssetAuthorization::Grant(_))),
                "expected {} to be covered, got {:?}",
                path,
                decision
            );
        }
    }

    /// The failure this whole module is shaped around: a sibling whose name
    /// merely STARTS with the granted one.
    #[test]
    fn a_sibling_that_shares_a_string_prefix_is_not_covered() {
        let token = mint_asset_grant(SECRET, &grant());
        for path in [
            "/photos-private/a.jpg",
            "/photosx",
            "/photos-private",
            "/other/photos/a.jpg",
        ] {
            assert_eq!(
                authorize_asset_read(
                    SECRET,
                    scope(path),
                    AssetCredential::Grant { token: &token }
                )
                .unwrap_err(),
                AssetAuthError::OutOfScope,
                "{} must not be covered by a grant for /photos",
                path
            );
        }
    }

    #[test]
    fn prefix_matching_is_segment_wise() {
        assert!(prefix_covers("/photos", "/photos/a.jpg"));
        assert!(prefix_covers("/photos", "photos/a.jpg"));
        assert!(prefix_covers("/photos/", "/photos/a/b/c.jpg"));
        assert!(prefix_covers("/photos", "/photos"));
        assert!(!prefix_covers("/photos", "/photos-private/a.jpg"));
        assert!(!prefix_covers("/photos", "/photosprivate"));
        assert!(!prefix_covers("/photos/a", "/photos/ab"));
        assert!(!prefix_covers("/photos/a", "/photos"));
    }

    #[test]
    fn the_workspace_root_covers_everything_in_the_workspace() {
        assert!(prefix_covers("/", "/anything/at/all.jpg"));
        assert!(prefix_covers("", "/anything.jpg"));
    }

    #[test]
    fn traversal_never_matches() {
        assert!(!prefix_covers("/photos", "/photos/../secrets/a.jpg"));
        assert!(!prefix_covers("/photos/..", "/secrets/a.jpg"));
        assert!(!prefix_covers("/photos", "/photos/./a.jpg"));
        assert!(normalize_path("/a/../b").is_none());
    }

    #[test]
    fn a_grant_whose_prefix_cannot_be_normalized_is_refused_as_a_token() {
        let mut g = grant();
        g.prefix = "/photos/..".into();
        let token = mint_asset_grant(SECRET, &g);
        assert_eq!(
            decode_grant(SECRET, &token).unwrap_err(),
            AssetAuthError::MalformedGrant
        );
    }

    #[test]
    fn a_grant_cannot_cross_tenant_repo_branch_or_workspace() {
        let token = mint_asset_grant(SECRET, &grant());
        let base = scope("/photos/a.jpg");

        let cases = [
            AssetReadScope {
                tenant_id: "tenant-b",
                ..base
            },
            AssetReadScope {
                repo: "other",
                ..base
            },
            AssetReadScope {
                branch: "publish",
                ..base
            },
            AssetReadScope {
                workspace: "stories",
                ..base
            },
        ];

        for case in cases {
            assert_eq!(
                authorize_asset_read(SECRET, case, AssetCredential::Grant { token: &token })
                    .unwrap_err(),
                AssetAuthError::OutOfScope
            );
        }
    }

    #[test]
    fn an_expired_grant_is_refused() {
        let mut g = grant();
        g.expires = 1;
        let token = mint_asset_grant(SECRET, &g);
        assert_eq!(
            authorize_asset_read(
                SECRET,
                scope("/photos/a.jpg"),
                AssetCredential::Grant { token: &token }
            )
            .unwrap_err(),
            AssetAuthError::Expired
        );
    }

    #[test]
    fn tampering_fails_closed() {
        let token = mint_asset_grant(SECRET, &grant());
        let mut parts = token.split('.');
        let (version, payload, signature) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );

        // A payload widened to the workspace root, re-encoded, old signature.
        let widened = AssetGrant {
            prefix: "/".into(),
            ..grant()
        };
        let widened_payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&widened).expect("serializes"));
        let forged = format!("{}.{}.{}", version, widened_payload, signature);
        assert_eq!(
            decode_grant(SECRET, &forged).unwrap_err(),
            AssetAuthError::BadSignature
        );

        // Signed with a secret this deployment does not hold.
        let foreign = mint_asset_grant(OTHER_SECRET, &widened);
        assert_eq!(
            decode_grant(SECRET, &foreign).unwrap_err(),
            AssetAuthError::BadSignature
        );

        // Shapes that are not tokens at all.
        for bad in [
            "",
            "rag1",
            "rag1.payload",
            "rag2.payload.signature",
            &format!("rag1.{}.{}.extra", payload, signature),
            &format!("rag1.not-base64!!.{}", signature),
        ] {
            assert!(
                matches!(
                    decode_grant(SECRET, bad),
                    Err(AssetAuthError::MalformedGrant) | Err(AssetAuthError::BadSignature)
                ),
                "{:?} must not decode",
                bad
            );
        }
    }

    /// Domain separation: a grant's HMAC and an asset URL's HMAC share a secret
    /// and must never be interchangeable.
    #[test]
    fn a_grant_signature_is_not_an_asset_url_signature() {
        let token = mint_asset_grant(SECRET, &grant());
        let grant_sig = token.rsplit('.').next().unwrap();

        let refused = authorize_asset_read(
            SECRET,
            scope("/photos/a.jpg"),
            AssetCredential::Signature {
                sig: grant_sig,
                expires: u64::MAX,
            },
        );
        assert_eq!(refused.unwrap_err(), AssetAuthError::BadSignature);
    }

    /// The single-asset form must keep working through the shared entry point,
    /// byte for byte as the old handler checked it.
    #[test]
    fn the_single_asset_signature_still_verifies_through_the_one_entry_point() {
        for property in ["file", "thumbnail"] {
            let expires = u64::MAX;
            let minted = crate::build_signed_asset_url(
                SECRET,
                "tenant-a",
                "media",
                "main",
                "assets",
                "/photos/a.jpg",
                property,
                "display",
                expires,
                None,
            );
            let sig = minted
                .url
                .split("sig=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .expect("minted URL carries a sig");

            let decision = authorize_asset_read(
                SECRET,
                AssetReadScope {
                    property,
                    ..scope("/photos/a.jpg")
                },
                AssetCredential::Signature { sig, expires },
            );
            assert!(matches!(decision, Ok(AssetAuthorization::Signature)));
        }
    }

    #[test]
    fn a_signature_for_one_asset_does_not_open_another() {
        let expires = u64::MAX;
        let minted = crate::build_signed_asset_url(
            SECRET,
            "tenant-a",
            "media",
            "main",
            "assets",
            "/photos/a.jpg",
            "file",
            "display",
            expires,
            None,
        );
        let sig = minted
            .url
            .split("sig=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .unwrap();

        assert_eq!(
            authorize_asset_read(
                SECRET,
                scope("/photos/b.jpg"),
                AssetCredential::Signature { sig, expires }
            )
            .unwrap_err(),
            AssetAuthError::BadSignature
        );
    }

    #[test]
    fn a_request_with_no_credential_is_refused() {
        assert_eq!(
            AssetCredential::from_query(None, None, None).unwrap_err(),
            AssetAuthError::MissingCredential
        );
        assert_eq!(
            AssetCredential::from_query(Some(""), Some(0), Some("")).unwrap_err(),
            AssetAuthError::MissingCredential
        );
    }

    #[test]
    fn a_grant_is_preferred_when_both_are_present() {
        let token = mint_asset_grant(SECRET, &grant());
        let credential = AssetCredential::from_query(Some("sig"), Some(1), Some(&token)).unwrap();
        assert!(matches!(credential, AssetCredential::Grant { .. }));
    }

    #[test]
    fn lifetimes_are_clamped() {
        assert_eq!(clamp_grant_lifetime(0), 1);
        assert_eq!(clamp_grant_lifetime(60), 60);
        assert_eq!(clamp_grant_lifetime(u64::MAX), MAX_GRANT_LIFETIME_SECS);
    }
}
