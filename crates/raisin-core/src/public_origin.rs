// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The PUBLIC origin of one tenant, derived from the engine's own configuration.
//!
//! One engine process serves many tenants, each at its own host
//! (`<handle>.rdb.example.com`). A function that must hand out an anonymous
//! address of a published asset (a mail picture, an Open Graph image) needs that
//! origin, and a single `RAISINDB_BASE_URL` cannot be right for all tenants.
//!
//! # Where the answer comes from (and where it never comes from)
//!
//! Operator configuration only:
//!
//! * `RAISINDB_PUBLIC_ORIGIN_TEMPLATE`, e.g. `https://{tenant_handle}.rdb.example.com`.
//!   Placeholders: `{tenant_handle}` (looked up in the handle map) and
//!   `{tenant_id}` (the immutable id the engine already knows).
//! * `RAISINDB_TENANT_HANDLE_MAP`: path of a JSON file
//!   `{"tenants":[{"handle":"acme","tenant_id":"<24 hex>"}, ...]}` (the shape
//!   the platform registry already serves to the edge). Re-read when its mtime
//!   changes, so a new tenant needs no engine restart.
//! * Otherwise `RAISINDB_BASE_URL` (single-tenant deployments).
//!
//! It is NEVER taken from a request header, a node property or a caller
//! argument: an editor or a visitor could forge any of those, and the result is
//! printed into mail that recipients trust. Unconfigured, unmapped, or malformed
//! configuration fails CLOSED with an error that names the setting.
//!
//! The answer depends on the tenant id alone, so it is identical for every
//! caller and every invocation path (trigger, flow, queue, HTTP).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Why a public origin could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicOriginError {
    /// Neither the template nor `RAISINDB_BASE_URL` is configured.
    NotConfigured,
    /// The template needs a handle but no map is configured or readable.
    HandleMapUnavailable(String),
    /// The tenant has no entry in the handle map.
    UnknownTenant,
    /// The configured template/base does not produce a plain http(s) origin.
    InvalidOrigin(String),
}

impl std::fmt::Display for PublicOriginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(
                f,
                "this server has no public origin configured; set \
                 RAISINDB_PUBLIC_ORIGIN_TEMPLATE (multi-tenant, e.g. \
                 https://{{tenant_handle}}.rdb.example.com) or RAISINDB_BASE_URL \
                 (single origin)"
            ),
            Self::HandleMapUnavailable(why) => write!(
                f,
                "RAISINDB_PUBLIC_ORIGIN_TEMPLATE uses {{tenant_handle}} but the tenant \
                 handle map is unavailable: {why} (set RAISINDB_TENANT_HANDLE_MAP)"
            ),
            Self::UnknownTenant => write!(
                f,
                "this tenant has no public host (not in the tenant handle map)"
            ),
            Self::InvalidOrigin(why) => {
                write!(f, "the configured public origin is not usable: {why}")
            }
        }
    }
}

impl std::error::Error for PublicOriginError {}

/// A handle is a DNS label the edge also accepts; anything else is refused so a
/// poisoned map cannot inject a path, port or second host into the origin.
fn is_valid_handle(h: &str) -> bool {
    let b = h.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

fn is_valid_tenant_id(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 64
        && t.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Parse the handle map JSON into `tenant_id -> handle`, dropping malformed rows.
pub fn parse_handle_map(json: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return out;
    };
    for t in v
        .get("tenants")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(h), Some(id)) = (
            t.get("handle").and_then(|x| x.as_str()),
            t.get("tenant_id").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        if is_valid_handle(h) && is_valid_tenant_id(id) {
            out.insert(id.to_string(), h.to_string());
        }
    }
    out
}

/// Pure resolver. `handle_of` is only consulted when the template needs it.
///
/// `template` wins over `base_url`: on a multi-tenant host a stray single
/// `RAISINDB_BASE_URL` must not make every tenant answer with one origin.
pub fn resolve_public_origin(
    template: Option<&str>,
    base_url: Option<&str>,
    tenant_id: &str,
    handle_of: &dyn Fn(&str) -> Result<Option<String>, PublicOriginError>,
) -> Result<String, PublicOriginError> {
    let template = template.map(str::trim).filter(|t| !t.is_empty());
    let raw = if let Some(tpl) = template {
        let mut s = tpl.to_string();
        if s.contains("{tenant_handle}") {
            let handle = handle_of(tenant_id)?.ok_or(PublicOriginError::UnknownTenant)?;
            if !is_valid_handle(&handle) {
                return Err(PublicOriginError::InvalidOrigin("bad handle".into()));
            }
            s = s.replace("{tenant_handle}", &handle);
        }
        if s.contains("{tenant_id}") {
            if !is_valid_tenant_id(tenant_id) {
                return Err(PublicOriginError::InvalidOrigin("bad tenant id".into()));
            }
            s = s.replace("{tenant_id}", tenant_id);
        }
        s
    } else if let Some(b) = base_url.map(str::trim).filter(|b| !b.is_empty()) {
        b.to_string()
    } else {
        return Err(PublicOriginError::NotConfigured);
    };
    validate_origin(&raw)
}

/// Only `scheme://host[:port]`, lowercased, no trailing slash, no path, query,
/// fragment or credentials.
fn validate_origin(raw: &str) -> Result<String, PublicOriginError> {
    let bad = |why: &str| Err(PublicOriginError::InvalidOrigin(why.to_string()));
    let raw = raw.trim().trim_end_matches('/');
    let Some((scheme, rest)) = raw.split_once("://") else {
        return bad("missing scheme");
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return bad("scheme must be http or https");
    }
    if rest.is_empty()
        || rest.contains(['/', '?', '#', '@', ' ', '\\', '{', '}'])
        || !rest
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_' | b':'))
    {
        return bad("must be scheme://host[:port] with nothing else");
    }
    Ok(format!("{scheme}://{}", rest.to_ascii_lowercase()))
}

struct MapCache {
    path: String,
    mtime: Option<SystemTime>,
    map: HashMap<String, String>,
}

fn map_cache() -> &'static Mutex<Option<MapCache>> {
    static C: OnceLock<Mutex<Option<MapCache>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// `tenant_id -> handle` from the configured map file, reloaded on mtime change.
fn handle_from_file(tenant_id: &str) -> Result<Option<String>, PublicOriginError> {
    let path = std::env::var("RAISINDB_TENANT_HANDLE_MAP")
        .ok()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| PublicOriginError::HandleMapUnavailable("not configured".into()))?;
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| PublicOriginError::HandleMapUnavailable(format!("{path}: {e}")))?;
    let mut guard = map_cache().lock().unwrap_or_else(|e| e.into_inner());
    let fresh = matches!(&*guard, Some(c) if c.path == path && c.mtime == Some(mtime));
    if !fresh {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| PublicOriginError::HandleMapUnavailable(format!("{path}: {e}")))?;
        *guard = Some(MapCache {
            path,
            mtime: Some(mtime),
            map: parse_handle_map(&text),
        });
    }
    Ok(guard.as_ref().and_then(|c| c.map.get(tenant_id).cloned()))
}

/// The public origin of `tenant_id` from process configuration. See module docs.
pub fn public_origin_for_tenant(tenant_id: &str) -> Result<String, PublicOriginError> {
    let template = std::env::var("RAISINDB_PUBLIC_ORIGIN_TEMPLATE").ok();
    let base = crate::asset_urls::configured_public_base_url();
    resolve_public_origin(
        template.as_deref(),
        base.as_deref(),
        tenant_id,
        &handle_from_file,
    )
}

/// Branches a public URL may be minted for unless the operator widens it via
/// `RAISINDB_PUBLIC_ASSET_BRANCHES` (comma-separated). Only `publish`: the
/// anonymous address must never expose working-copy (`main`) content.
pub fn public_asset_branch_allowed(branch: &str) -> bool {
    if branch == "publish" {
        return true;
    }
    std::env::var("RAISINDB_PUBLIC_ASSET_BRANCHES")
        .map(|v| {
            v.split(',')
                .any(|b| b.trim() == branch && !b.trim().is_empty())
        })
        .unwrap_or(false)
}

/// Validate a workspace-relative asset path: absolute, normalised, no `.`/`..`
/// segments, no empty segments, no query/fragment/control/backslash characters.
pub fn validate_public_asset_path(path: &str) -> Result<(), &'static str> {
    if !path.starts_with('/') || path == "/" {
        return Err("path must be absolute and name an asset");
    }
    if path.contains(['?', '#', '\\', '%']) || path.chars().any(|c| c.is_control()) {
        return Err("path must not contain ?, #, %, backslash or control characters");
    }
    for seg in path[1..].split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err("path must not contain empty, '.' or '..' segments");
        }
    }
    Ok(())
}

/// Percent-encode one path segment (RFC 3986 unreserved kept).
fn encode_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `<origin>/api/repository/<repo>/<branch>/head/<workspace><path>/raisin:display`
/// with no query string: the anonymous, published form.
pub fn build_public_asset_url(
    origin: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> String {
    let encoded_path: String = path[1..]
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "{origin}/api/repository/{}/{}/head/{}/{}/raisin:display",
        encode_segment(repo),
        encode_segment(branch),
        encode_segment(workspace),
        encoded_path
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const T_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaa";
    const T_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbb";
    const TPL: &str = "https://{tenant_handle}.rdb.example.com";

    fn map() -> HashMap<String, String> {
        parse_handle_map(&format!(
            r#"{{"tenants":[{{"handle":"acme","tenant_id":"{T_A}"}},
                {{"handle":"solutas","tenant_id":"{T_B}"}},
                {{"handle":"evil/../x","tenant_id":"cccccccccccccccccccccccc"}},
                {{"handle":"EVIL","tenant_id":"dddddddddddddddddddddddd"}}]}}"#
        ))
    }

    fn lookup(id: &str) -> Result<Option<String>, PublicOriginError> {
        Ok(map().get(id).cloned())
    }

    #[test]
    fn two_tenants_get_two_origins() {
        let a = resolve_public_origin(Some(TPL), None, T_A, &lookup).unwrap();
        let b = resolve_public_origin(Some(TPL), None, T_B, &lookup).unwrap();
        assert_eq!(a, "https://acme.rdb.example.com");
        assert_eq!(b, "https://solutas.rdb.example.com");
    }

    #[test]
    fn unconfigured_fails_closed() {
        assert_eq!(
            resolve_public_origin(None, None, T_A, &lookup),
            Err(PublicOriginError::NotConfigured)
        );
        assert_eq!(
            resolve_public_origin(Some("  "), Some(""), T_A, &lookup),
            Err(PublicOriginError::NotConfigured)
        );
    }

    #[test]
    fn unmapped_tenant_and_poisoned_handles_fail_closed() {
        assert_eq!(
            resolve_public_origin(Some(TPL), None, "ffffffffffffffffffffffff", &lookup),
            Err(PublicOriginError::UnknownTenant)
        );
        // rows with a hostile/uppercase handle were dropped at parse time
        assert!(!map().contains_key("cccccccccccccccccccccccc"));
        assert!(!map().contains_key("dddddddddddddddddddddddd"));
    }

    #[test]
    fn missing_map_fails_closed_when_template_needs_it() {
        let none = |_: &str| Err(PublicOriginError::HandleMapUnavailable("x".into()));
        assert!(matches!(
            resolve_public_origin(Some(TPL), None, T_A, &none),
            Err(PublicOriginError::HandleMapUnavailable(_))
        ));
        // a template without {tenant_handle} never touches the map
        assert_eq!(
            resolve_public_origin(Some("https://{tenant_id}.x.test"), None, T_A, &none).unwrap(),
            format!("https://{T_A}.x.test")
        );
    }

    #[test]
    fn template_beats_base_url_and_base_url_is_the_single_tenant_fallback() {
        assert_eq!(
            resolve_public_origin(Some(TPL), Some("https://one.test"), T_A, &lookup).unwrap(),
            "https://acme.rdb.example.com"
        );
        assert_eq!(
            resolve_public_origin(None, Some("https://One.test/"), T_A, &lookup).unwrap(),
            "https://one.test"
        );
    }

    #[test]
    fn origin_must_be_plain() {
        for bad in [
            "ftp://a.test",
            "a.test",
            "https://a.test/path",
            "https://a.test?x=1",
            "https://u@a.test",
            "https://a.test#f",
        ] {
            assert!(
                matches!(
                    resolve_public_origin(Some(bad), None, T_A, &lookup),
                    Err(PublicOriginError::InvalidOrigin(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn path_validation() {
        assert!(validate_public_asset_path("/public/mail/hero.jpg").is_ok());
        for bad in [
            "",
            "/",
            "a/b",
            "/a/../b",
            "/a/./b",
            "/a//b",
            "/a/b?x=1",
            "/a/b#f",
            "/a\\b",
            "/a/%2e%2e/b",
            "/a/b\n",
        ] {
            assert!(validate_public_asset_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn url_shape_has_no_query_and_encodes_segments() {
        assert_eq!(
            build_public_asset_url(
                "https://solutas.rdb.example.com",
                "studio",
                "publish",
                "assets",
                "/public/My Pic (1).jpg"
            ),
            "https://solutas.rdb.example.com/api/repository/studio/publish/head/assets/public/My%20Pic%20%281%29.jpg/raisin:display"
        );
    }

    #[test]
    fn only_publish_branch_by_default() {
        assert!(public_asset_branch_allowed("publish"));
        assert!(!public_asset_branch_allowed("main"));
    }
}
