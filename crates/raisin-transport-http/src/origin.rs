// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What is my own public origin?
//!
//! The one answer to that question, for every place RaisinDB has to build a URL
//! that points back at itself — an OAuth issuer, an MCP resource id, a signed
//! asset URL, and the verify link in a magic-link email.
//!
//! # Why it needs to be one answer
//!
//! It was six. The magic-link email was built from the tenant's `/config/email`
//! `base_url`, which is that tenant's **front end**, with a RaisinDB route
//! appended — so the emailed link pointed at the customer's own site and 404ed.
//! Meanwhile the OAuth issuer derived it per request with a proxy gate and a
//! host allowlist, the MCP handler derived it from `Host` with neither, and
//! three more callers read `RAISINDB_BASE_URL` raw and fell back to the empty
//! string.
//!
//! Two distinct questions were being answered by one field, which is the whole
//! bug. THIS module answers "where am I". "May I send a browser to this URL?"
//! is a different question with a different answer — see `resolve_redirect` in
//! `handlers/identity_auth/magic_link.rs`, which keeps an explicit per-tenant
//! allowlist. Do not merge them.
//!
//! # Why the host is not simply believed
//!
//! `Host` is attacker-controlled. For a link carrying a one-time sign-in token
//! that is pre-auth account takeover: post a magic-link request with
//! `Host: evil.test`, and the victim is emailed a working token pointed at the
//! attacker's server. So a derived host is validated, and
//! [`OriginTrust::Required`] refuses rather than guessing.

use std::sync::Once;

use axum::http::HeaderMap;

/// How hard to insist on an allowlist when the origin is derived from a header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginTrust {
    /// Refuse when no allowlist is configured.
    ///
    /// For anything that puts the resulting URL in front of a user who will
    /// trust it — an emailed link above all. Not sending is a better failure
    /// than sending a poisoned link.
    Required,
    /// Accept an unvalidated derived origin, warning once.
    ///
    /// Preserves the OAuth issuer's long-standing single-origin behaviour for
    /// deployments that configure nothing. Tightening that is a separate,
    /// deliberate change — it would break any such deployment on upgrade.
    WarnOnce,
}

/// Why an origin could not be established.
#[derive(Debug, Clone)]
pub enum OriginError {
    /// A host was derived but is not on any allowlist.
    UntrustedHost(String),
    /// Nothing is configured and the caller demanded certainty.
    NotConfigured,
}

impl std::fmt::Display for OriginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OriginError::UntrustedHost(host) => write!(
                f,
                "request host `{host}` is not an allowed origin for this deployment; \
                 add it to RAISINDB_TRUSTED_HOST_SUFFIXES or the tenant's trusted hosts"
            ),
            OriginError::NotConfigured => write!(
                f,
                "this deployment has no trusted origin configured; set RAISINDB_BASE_URL \
                 for a single fixed origin, or RAISINDB_TRUSTED_HOST_SUFFIXES for a \
                 multi-tenant one"
            ),
        }
    }
}

/// The externally-visible origin of THIS server, for the current request.
///
/// Order: the `RAISINDB_BASE_URL` override, else the forwarded host (only
/// behind a trusted proxy), else `Host`. A derived host is then checked against
/// `RAISINDB_TRUSTED_HOST_SUFFIXES`, the per-tenant hosts the caller passes, and
/// loopback.
///
/// Per request rather than a process global on purpose: one process serves many
/// orgs at `{handle}.example.com`, and a single configured value cannot be right
/// for all of them.
///
/// The result never has a trailing slash and is canonicalised, so two spellings
/// of one origin compare equal.
pub fn self_origin(
    headers: &HeaderMap,
    tenant_hosts: &[String],
    trust: OriginTrust,
) -> Result<String, OriginError> {
    if let Some(base) = configured_base_url() {
        return Ok(canonical_origin(&base));
    }

    let trust_forwarded = forwarded_headers_trusted();
    let proto = trust_forwarded
        .then(|| header_str(headers, "x-forwarded-proto"))
        .flatten()
        .unwrap_or("https");
    let host = trust_forwarded
        .then(|| header_str(headers, "x-forwarded-host"))
        .flatten()
        .or_else(|| header_str(headers, "host"))
        .unwrap_or("localhost");

    let origin = canonical_origin(&format!("{proto}://{host}"));
    let suffixes = trusted_host_suffixes();

    if suffixes.is_empty() && tenant_hosts.is_empty() {
        return match trust {
            OriginTrust::WarnOnce => {
                warn_unconfigured_origin_once();
                Ok(origin)
            }
            OriginTrust::Required => Err(OriginError::NotConfigured),
        };
    }

    if host_is_trusted(host, &suffixes, tenant_hosts) {
        Ok(origin)
    } else {
        Err(OriginError::UntrustedHost(host.to_string()))
    }
}

/// The configured canonical base URL, if `RAISINDB_BASE_URL` is set non-empty.
pub(crate) fn configured_base_url() -> Option<String> {
    std::env::var("RAISINDB_BASE_URL")
        .ok()
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
}

/// Host suffixes trusted for every tenant (`RAISINDB_TRUSTED_HOST_SUFFIXES`,
/// comma-separated).
pub(crate) fn trusted_host_suffixes() -> Vec<String> {
    std::env::var("RAISINDB_TRUSTED_HOST_SUFFIXES")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the derived host is on the allowlist (loopback, a per-tenant exact
/// host, or a trusted suffix). A port is ignored for the comparison.
pub(crate) fn host_is_trusted(host: &str, suffixes: &[String], tenant_hosts: &[String]) -> bool {
    let hostname = host.split(':').next().unwrap_or(host);

    if matches!(hostname, "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    if tenant_hosts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host) || h.eq_ignore_ascii_case(hostname))
    {
        return true;
    }
    suffixes.iter().any(|s| matches_host_suffix(hostname, s))
}

/// A suffix matches its apex and any subdomain: `example.com` matches
/// `example.com` and `a.example.com`, but not `evilexample.com`.
pub(crate) fn matches_host_suffix(hostname: &str, suffix: &str) -> bool {
    let suffix = suffix.trim().trim_start_matches('.').to_ascii_lowercase();
    if suffix.is_empty() {
        return false;
    }
    let host = hostname.to_ascii_lowercase();
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

/// Whether `X-Forwarded-*` headers may be trusted (a trusted proxy sets them).
///
/// Off unless `RAISINDB_TRUST_FORWARDED_HEADERS` is `1`/`true`/`yes`/`on`.
pub(crate) fn forwarded_headers_trusted() -> bool {
    std::env::var("RAISINDB_TRUST_FORWARDED_HEADERS")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn warn_unconfigured_origin_once() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "No origin allowlist configured (RAISINDB_TRUSTED_HOST_SUFFIXES unset and no \
             per-tenant trusted hosts); self-referential URLs are derived from the request \
             host without validation. Set RAISINDB_TRUSTED_HOST_SUFFIXES (or \
             RAISINDB_BASE_URL for a single fixed origin) in any proxied or multi-origin \
             deployment."
        );
    });
}

/// Read a header value as a borrowed `str`, taking the first comma-separated
/// token (proxy headers may carry a list).
pub(crate) fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or(v).trim())
        .filter(|v| !v.is_empty())
}

/// Canonicalize an origin (`scheme://host[:port]`) so two spellings of the same
/// origin compare equal.
///
/// The scheme and host are lowercased, a default port is dropped (`:443` on
/// `https`, `:80` on `http`), and any trailing slash is removed. This exists
/// because a token audience is *minted* from one derivation of the origin and
/// *verified* against another; without canonicalization a trailing slash, an
/// explicit `:443`, or an uppercase host silently breaks every MCP request with
/// an audience mismatch that looks, from the outside, like a permissions bug.
pub(crate) fn canonical_origin(origin: &str) -> String {
    let origin = origin.trim().trim_end_matches('/');
    let (scheme, rest) = match origin.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        // No scheme to normalize; fall back to the input lowercased as a host.
        None => return origin.to_ascii_lowercase(),
    };

    let authority = rest.to_ascii_lowercase();
    let authority = match (scheme.as_str(), authority.rsplit_once(':')) {
        // Only strip a *default* port, and only when what follows the colon is
        // actually a port (an IPv6 literal ends in `]`).
        ("https", Some((host, "443"))) | ("http", Some((host, "80"))) if !host.is_empty() => {
            host.to_string()
        }
        _ => authority,
    };

    format!("{scheme}://{authority}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    /// The whole point of `Required`: with nothing configured we must refuse,
    /// not guess. A guessed origin in a magic-link email is a token handed to
    /// whoever set the Host header.
    #[test]
    fn required_refuses_when_nothing_is_configured() {
        let h = headers(&[("host", "evil.test")]);
        let err = self_origin(&h, &[], OriginTrust::Required)
            .expect_err("no allowlist must refuse under Required");
        assert!(matches!(err, OriginError::NotConfigured));
        // And the message tells the operator what to set.
        assert!(err.to_string().contains("RAISINDB_TRUSTED_HOST_SUFFIXES"));
    }

    /// A per-tenant host is an allowlist, so it flips Required from "refuse"
    /// to "check" — and an unlisted host is then refused by name.
    #[test]
    fn an_unlisted_host_is_refused_by_name() {
        let h = headers(&[("host", "evil.test")]);
        let err = self_origin(&h, &["acme.example.com".into()], OriginTrust::Required)
            .expect_err("an unlisted host must be refused");
        match err {
            OriginError::UntrustedHost(host) => assert_eq!(host, "evil.test"),
            other => panic!("expected UntrustedHost, got {other:?}"),
        }
    }

    #[test]
    fn a_tenant_host_is_accepted() {
        let h = headers(&[("host", "acme.example.com")]);
        let origin = self_origin(&h, &["acme.example.com".into()], OriginTrust::Required)
            .expect("a listed tenant host is allowed");
        assert_eq!(origin, "https://acme.example.com");
    }

    /// A forwarded host is IGNORED unless the deployment says a proxy sets it.
    /// Otherwise anyone could present one directly to the listener.
    #[test]
    fn a_forwarded_host_is_ignored_without_the_trust_flag() {
        let h = headers(&[
            ("host", "acme.example.com"),
            ("x-forwarded-host", "evil.test"),
        ]);
        // RAISINDB_TRUST_FORWARDED_HEADERS is unset in the test environment.
        let origin = self_origin(&h, &["acme.example.com".into()], OriginTrust::Required)
            .expect("the real Host is used");
        assert_eq!(origin, "https://acme.example.com");
    }

    #[test]
    fn origins_are_canonicalised() {
        assert_eq!(
            canonical_origin("HTTPS://Acme.Example.com:443/"),
            "https://acme.example.com"
        );
        assert_eq!(
            canonical_origin("http://acme.example.com:80"),
            "http://acme.example.com"
        );
        assert_eq!(
            canonical_origin("https://acme.example.com:8443"),
            "https://acme.example.com:8443"
        );
    }
}
