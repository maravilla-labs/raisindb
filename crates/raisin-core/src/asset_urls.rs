// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The ONE composer of a signed asset URL.
//!
//! [`sign_asset_url`](crate::sign_asset_url) signs a string; it does not say
//! WHICH string. That grammar — `{repo}/{branch}/head/{ws}{path}[@{prop}]`, the
//! `@prop` suffix omitted for `file`, and the `/api/repository/…/raisin:{cmd}`
//! route it hangs off — lived in three hand-rolled copies: the HTTP sign
//! handler, the HTTP serve handler that verifies, and now a function binding
//! that mints a URL for an out-of-process media service.
//!
//! A minter and a verifier that disagree by one byte do not fail visibly. They
//! produce a URL that always answers 401 INVALID_SIGNATURE, with nothing
//! anywhere naming the difference — the signature is a hash, so there is no
//! diff to read. That is why the grammar is written down once here and both
//! sides call it.
//!
//! # What is deliberately NOT here
//!
//! The SECRET. `raisin-core` does not depend on `raisin-crypto`, and pushing
//! the reader down here would mean two ways to obtain the key. Callers pass
//! bytes; `raisin_crypto::signing_secret_or_dev()` is where they come from.

/// The asset property a signed URL addresses when the caller names none.
///
/// Load-bearing in the signature, not just a default: for `file` the `@prop`
/// suffix is omitted from the signed path AND `None` is passed as the property
/// to the HMAC. Both sides of the wire must make that same exception or every
/// `file` URL 401s.
pub const DEFAULT_ASSET_PROPERTY: &str = "file";

/// Build the string that gets signed for an asset read.
///
/// `{repo}/{branch}/head/{workspace}{node_path}` plus `@{property}` unless the
/// property is [`DEFAULT_ASSET_PROPERTY`]. `head` is the revision locator: a
/// signed URL always addresses the current revision, never a pinned one.
pub fn signed_asset_path(
    repo: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    property: &str,
) -> String {
    // A path with no leading slash and one with it must produce the SAME signed
    // string, or a caller that spells it differently from the verifier gets a
    // URL that cannot be redeemed.
    let node_path = if node_path.starts_with('/') {
        node_path.to_string()
    } else {
        format!("/{}", node_path)
    };

    if property == DEFAULT_ASSET_PROPERTY {
        format!("{}/{}/head/{}{}", repo, branch, workspace, node_path)
    } else {
        format!(
            "{}/{}/head/{}{}@{}",
            repo, branch, workspace, node_path, property
        )
    }
}

/// The property as the HMAC takes it: `None` for `file`.
///
/// Kept as a function so the exception cannot be spelled out on one side of the
/// wire and forgotten on the other.
pub fn signature_property(property: &str) -> Option<&str> {
    if property == DEFAULT_ASSET_PROPERTY {
        None
    } else {
        Some(property)
    }
}

/// A minted signed URL and the moment it stops working.
#[derive(Debug, Clone)]
pub struct SignedAssetUrl {
    /// The URL to hand out. Absolute when a base was supplied, otherwise
    /// root-relative.
    pub url: String,
    /// Unix seconds at which the signature expires.
    pub expires: u64,
    /// The string that was signed — useful in a log line when a URL is rejected.
    pub signed_path: String,
}

/// Mint a complete signed asset URL.
///
/// `base_url` is the deployment's own public origin, already trimmed of its
/// trailing slash (see [`configured_public_base_url`]). `None` yields a
/// root-relative URL, which is what the browser-facing HTTP endpoint has always
/// returned; a consumer that is another PROCESS must pass one, because a
/// relative path is not resolvable there.
#[allow(clippy::too_many_arguments)]
pub fn build_signed_asset_url(
    secret: &[u8],
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    property: &str,
    command: &str,
    expires: u64,
    base_url: Option<&str>,
) -> SignedAssetUrl {
    build_signed_asset_url_bound(
        secret, tenant_id, repo, branch, workspace, node_path, property, command, expires,
        base_url, None,
    )
}

/// [`build_signed_asset_url`], bound to the blob's content when the
/// property's envelope records a sha256 (see [`content_sha256_of`]).
#[allow(clippy::too_many_arguments)]
pub fn build_signed_asset_url_bound(
    secret: &[u8],
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    property: &str,
    command: &str,
    expires: u64,
    base_url: Option<&str>,
    content_sha256: Option<&str>,
) -> SignedAssetUrl {
    let signed_path = signed_asset_path(repo, branch, workspace, node_path, property);
    let signature = crate::sign_asset_url_bound(
        secret,
        tenant_id,
        &signed_path,
        command,
        signature_property(property),
        expires,
        content_sha256,
    );

    let url = format!(
        "{}/api/repository/{}/raisin:{}?sig={}&exp={}",
        base_url.unwrap_or(""),
        signed_path,
        command,
        signature,
        expires
    );

    SignedAssetUrl {
        url,
        expires,
        signed_path,
    }
}

/// The content digest a Resource envelope records (`metadata.sha256`), if any.
///
/// Only generated artifacts carry one today (a package build writes it from
/// the stream that produced the blob). Its presence is what makes a signed
/// URL content-bound and what marks the blob as an artifact for the longer
/// download lifetime; an ordinary upload has none and keeps both historical
/// behaviours.
pub fn content_sha256_of(
    resource: &raisin_models::nodes::properties::value::Resource,
) -> Option<&str> {
    match resource.metadata.as_ref()?.get("sha256")? {
        raisin_models::nodes::properties::PropertyValue::String(s)
            if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Some(s.as_str())
        }
        _ => None,
    }
}

/// Default lifetime of a signed download of an artifact: long enough to
/// finish — and resume — a multi-gigabyte transfer on the same URL.
pub const DEFAULT_ARTIFACT_URL_TTL_SECS: u64 = 12 * 3600;
/// Upper bound for an artifact download's lifetime, whatever is asked for.
pub const MAX_ARTIFACT_URL_TTL_SECS: u64 = 24 * 3600;

/// The artifact download lifetime: `RAISIN_ARTIFACT_URL_TTL_SECS`, else
/// [`DEFAULT_ARTIFACT_URL_TTL_SECS`], clamped to [`MAX_ARTIFACT_URL_TTL_SECS`].
pub fn artifact_url_ttl_secs() -> u64 {
    std::env::var("RAISIN_ARTIFACT_URL_TTL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_ARTIFACT_URL_TTL_SECS)
        .min(MAX_ARTIFACT_URL_TTL_SECS)
}

/// The deployment's canonical public base URL, or `None` when unconfigured.
///
/// `RAISINDB_BASE_URL` is the existing setting for "where am I" — the OAuth
/// issuer, the MCP resource id and the magic-link email all resolve through it.
/// Read here rather than at each call site because an empty value must count as
/// unset: an empty base silently produces a RELATIVE URL, which for an
/// out-of-process consumer is not a degraded answer but a broken one.
pub fn configured_public_base_url() -> Option<String> {
    std::env::var("RAISINDB_BASE_URL")
        .ok()
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
}

/// `true` when `command` is one the asset routes serve.
pub fn is_valid_asset_command(command: &str) -> bool {
    command == "download" || command == "display"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_asset_signature;

    const SECRET: &[u8] = b"test-secret-key-32-bytes-long!!!";

    #[test]
    fn file_property_is_omitted_from_the_signed_path() {
        assert_eq!(
            signed_asset_path("media", "main", "assets", "/a/b.jpg", "file"),
            "media/main/head/assets/a/b.jpg"
        );
        assert_eq!(
            signed_asset_path("media", "main", "assets", "/a/b.jpg", "thumbnail"),
            "media/main/head/assets/a/b.jpg@thumbnail"
        );
    }

    #[test]
    fn a_missing_leading_slash_signs_the_same_string() {
        assert_eq!(
            signed_asset_path("media", "main", "assets", "a/b.jpg", "file"),
            signed_asset_path("media", "main", "assets", "/a/b.jpg", "file")
        );
    }

    /// The whole reason this module exists: what the builder mints must be what
    /// the serve handler's verifier accepts, for BOTH the `file` exception and
    /// a named property.
    #[test]
    fn a_minted_url_verifies_the_way_the_serve_handler_checks_it() {
        for property in ["file", "thumbnail"] {
            let expires = u64::MAX;
            let minted = build_signed_asset_url(
                SECRET,
                "tenant-a",
                "media",
                "main",
                "assets",
                "/a/b.jpg",
                property,
                "display",
                expires,
                Some("https://db.example.test"),
            );

            let sig = minted
                .url
                .split("sig=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .expect("minted URL carries a sig parameter");

            assert!(verify_asset_signature(
                SECRET,
                "tenant-a",
                &minted.signed_path,
                "display",
                signature_property(property),
                expires,
                sig,
            ));
        }
    }

    #[test]
    fn an_absolute_base_is_prefixed_and_no_base_stays_relative() {
        let with_base = build_signed_asset_url(
            SECRET,
            "t",
            "media",
            "main",
            "assets",
            "/a.jpg",
            "file",
            "download",
            1,
            Some("https://db.example.test"),
        );
        assert!(with_base
            .url
            .starts_with("https://db.example.test/api/repository/media/main/head/assets/a.jpg"));

        let relative = build_signed_asset_url(
            SECRET, "t", "media", "main", "assets", "/a.jpg", "file", "download", 1, None,
        );
        assert!(relative.url.starts_with("/api/repository/"));
    }
}
