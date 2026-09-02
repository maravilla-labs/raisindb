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

use std::collections::HashMap;
use std::sync::{Mutex, Once, OnceLock};

use axum::http::HeaderMap;

/// The last PUBLIC origin each tenant was actually served on.
///
/// A tenant's own host is not derivable from anything RaisinDB stores: a
/// request that bypasses the proxy carries a tenant id, and nothing maps a
/// tenant id to a host — the handle-to-tenant map lives in the edge, one
/// direction only. So rather than require every co-located caller to be told
/// the public origin (which is per-app configuration that a new org silently
/// misses), the server LEARNS it from the requests that already carry it.
///
/// Only an origin that passed the allowlist is ever recorded, so this grants no
/// trust the request did not already have: it can only remember a host the
/// deployment was willing to serve for that tenant a moment ago.
///
/// Process-local and unpersisted, deliberately. It is a cache, not
/// configuration — after a restart the first proxied request refills it, and
/// until then a link that cannot be built is refused rather than guessed.
fn learned_origins() -> &'static Mutex<HashMap<String, String>> {
    static LEARNED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    LEARNED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember the origin this tenant was served on, for a later request that
/// arrives without one.
fn remember_origin(tenant_id: &str, origin: &str) {
    if let Ok(mut map) = learned_origins().lock() {
        match map.get(tenant_id) {
            Some(existing) if existing == origin => {}
            _ => {
                map.insert(tenant_id.to_string(), origin.to_string());
            }
        }
    }
}

/// Record the public origin a request was served on for this tenant, if the
/// request actually carries one.
///
/// Called from the tenant middleware, so EVERY proxied request teaches it —
/// not just the handful of endpoints that build a self-referential URL. Those
/// are rare (an OAuth issuer, an MCP resource id, a magic link), and learning
/// only from them would leave the map empty on exactly the deployment that
/// needs it: one whose app talks to RaisinDB over loopback and whose visitors
/// only ever hit ordinary data routes.
///
/// A loopback host teaches nothing — it is the case being solved, not evidence
/// of anything. Neither does an unvalidated one: the host must pass the same
/// allowlist a link would be checked against, or a spoofed `Host` on any
/// request could poison the origin a later magic link is built on.
pub fn note_request_origin(headers: &HeaderMap, tenant_id: &str) {
    // An absolute override makes learning pointless: it wins everywhere.
    if configured_base_url().is_some() {
        return;
    }
    let trust_forwarded = forwarded_headers_trusted();
    let proto = trust_forwarded
        .then(|| header_str(headers, "x-forwarded-proto"))
        .flatten()
        .unwrap_or("https");
    let Some(host) = trust_forwarded
        .then(|| header_str(headers, "x-forwarded-host"))
        .flatten()
        .or_else(|| header_str(headers, "host"))
    else {
        return;
    };
    if is_loopback(host) {
        return;
    }
    let suffixes = trusted_host_suffixes();
    if suffixes.is_empty() {
        // Nothing to validate against. Recording here would let any `Host`
        // become this tenant's origin.
        return;
    }
    if !host_is_trusted(host, &suffixes, &[]) {
        return;
    }
    remember_origin(tenant_id, &canonical_origin(&format!("{proto}://{host}")));
}

/// The origin this tenant was last served on, if one has been seen.
fn recall_origin(tenant_id: &str) -> Option<String> {
    learned_origins()
        .lock()
        .ok()
        .and_then(|m| m.get(tenant_id).cloned())
}

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
    /// The request arrived on loopback, and the result would be shown to
    /// somebody who is not on this machine.
    LoopbackNotPublic(String),
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
            OriginError::LoopbackNotPublic(host) => write!(
                f,
                "the request arrived on `{host}`, which nobody outside this machine can \
                 reach, and this deployment has a public identity configured; a link built \
                 on it would be dead on arrival. Give the tenant a trusted host so the \
                 public origin can be resolved, or have the caller forward the public host \
                 (X-Forwarded-Host) — a forwarded host is still checked against the \
                 allowlist"
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
    tenant_id: Option<&str>,
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

    // LOOPBACK IS NOT A PUBLIC IDENTITY.
    //
    // A magic link built on `https://127.0.0.1:8088/...` is dead the moment it
    // leaves the machine, and that is exactly what shipped: the tenant's app
    // runs beside RaisinDB and reaches it on the loopback address the platform
    // injects, so `Host` was `127.0.0.1:8088` and the allowlist waved it
    // through — loopback was unconditionally trusted. The recipient got a link
    // to their own computer.
    //
    // Trusting loopback is right on a developer's machine, where nothing else
    // is configured; that case returned above. Once a deployment has declared a
    // public identity, a loopback request is a local caller, not the public
    // origin, and the two answers must not be the same value.
    if is_loopback(host) {
        // The tenant named exactly one host, so there is nothing to guess.
        // Configuration wins over anything learned: it is what an operator
        // asserted, and it survives a restart.
        if let [only] = tenant_hosts {
            return Ok(canonical_origin(&format!("https://{only}")));
        }
        // Otherwise use the origin this tenant was last actually served on.
        // This is what lets a co-located app — one reaching RaisinDB on the
        // loopback address its platform injects — send a magic link without
        // being told the public host. Anyone browsing the site refills it.
        if let Some(learned) = tenant_id.and_then(recall_origin) {
            tracing::debug!(
                tenant_id = tenant_id.unwrap_or_default(),
                origin = %learned,
                "loopback request: using the origin this tenant was last served on"
            );
            return Ok(learned);
        }
        return match trust {
            // An OAuth issuer served over loopback is a local development
            // client talking to a local server — the URL is used in that same
            // process, not emailed to anyone. Tightening it here would break
            // those on upgrade, which is the same reason WarnOnce exists.
            OriginTrust::WarnOnce => Ok(origin),
            OriginTrust::Required => Err(OriginError::LoopbackNotPublic(host.to_string())),
        };
    }

    if host_is_trusted(host, &suffixes, tenant_hosts) {
        // Validated and public: remember it, so a later loopback request for
        // this tenant has a real answer instead of a refusal.
        if let Some(tenant_id) = tenant_id {
            remember_origin(tenant_id, &origin);
        }
        Ok(origin)
    } else {
        Err(OriginError::UntrustedHost(host.to_string()))
    }
}

/// Whether a host (with or without a port) addresses this machine.
///
/// The port split is not a plain `split(':')`: a bare IPv6 literal is all
/// colons, so splitting on the first one turns `::1` into the empty string and
/// splitting on the last turns it into `::`. Neither matches, and loopback
/// silently stops being recognised — which here would mean an emailed link
/// built on it. A trailing `:digits` is a port only when the host is bracketed
/// or has no other colon.
pub(crate) fn is_loopback(host: &str) -> bool {
    matches!(
        hostname_of(host).as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// The host part of an authority, with any port and IPv6 brackets removed.
pub(crate) fn hostname_of(authority: &str) -> String {
    let authority = authority.trim();
    let bare = match authority.rsplit_once(':') {
        Some((head, tail))
            if !tail.is_empty()
                && tail.bytes().all(|b| b.is_ascii_digit())
                && (head.ends_with(']') || !head.contains(':')) =>
        {
            head
        }
        _ => authority,
    };
    bare.trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

/// The configured canonical base URL, if `RAISINDB_BASE_URL` is set non-empty.
///
/// Delegates to `raisin-core` because the signed-URL builder lives there and
/// needs the same answer: `raisin-functions` cannot depend on this transport,
/// so a copy of the read would have been the sixth one this module exists to
/// have removed.
pub(crate) fn configured_base_url() -> Option<String> {
    raisin_core::configured_public_base_url()
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
    // One hostname parser, shared with `is_loopback`. The old local
    // `split(':').next()` disagreed with it on every IPv6 literal.
    let hostname = hostname_of(host);

    if is_loopback(host) {
        return true;
    }
    if tenant_hosts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host) || h.eq_ignore_ascii_case(&hostname))
    {
        return true;
    }
    suffixes.iter().any(|s| matches_host_suffix(&hostname, s))
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

    /// `RAISINDB_TRUSTED_HOST_SUFFIXES` is process-wide, so a test that sets it
    /// races every other test that reads it. Anything touching it takes this.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    /// Sets the suffix allowlist for the duration of `f`, then restores it.
    /// Restores on unwind too — a panicking assertion must not leave the
    /// variable set for whatever runs next.
    fn with_suffixes<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("RAISINDB_TRUSTED_HOST_SUFFIXES").ok();
        match value {
            Some(v) => std::env::set_var("RAISINDB_TRUSTED_HOST_SUFFIXES", v),
            None => std::env::remove_var("RAISINDB_TRUSTED_HOST_SUFFIXES"),
        }
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        match previous {
            Some(v) => std::env::set_var("RAISINDB_TRUSTED_HOST_SUFFIXES", v),
            None => std::env::remove_var("RAISINDB_TRUSTED_HOST_SUFFIXES"),
        }
        match out {
            Ok(v) => v,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

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
        let err = self_origin(&h, None, &[], OriginTrust::Required)
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
        let err = self_origin(
            &h,
            None,
            &["acme.example.com".into()],
            OriginTrust::Required,
        )
        .expect_err("an unlisted host must be refused");
        match err {
            OriginError::UntrustedHost(host) => assert_eq!(host, "evil.test"),
            other => panic!("expected UntrustedHost, got {other:?}"),
        }
    }

    #[test]
    fn a_tenant_host_is_accepted() {
        let h = headers(&[("host", "acme.example.com")]);
        let origin = self_origin(
            &h,
            None,
            &["acme.example.com".into()],
            OriginTrust::Required,
        )
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
        let origin = self_origin(
            &h,
            None,
            &["acme.example.com".into()],
            OriginTrust::Required,
        )
        .expect("the real Host is used");
        assert_eq!(origin, "https://acme.example.com");
    }

    /// The bug: a co-located app reaches RaisinDB on the loopback address the
    /// platform injects, so `Host` is `127.0.0.1:8088`. Loopback was
    /// unconditionally trusted, so the magic link was built on it and the
    /// recipient was emailed a link to their own computer.
    #[test]
    fn a_loopback_host_cannot_produce_an_emailed_link_on_a_configured_deployment() {
        let h = headers(&[("host", "127.0.0.1:8088")]);
        let err = self_origin(
            &h,
            None,
            &["acme.example.com".into(), "www.acme.test".into()],
            OriginTrust::Required,
        )
        .expect_err("loopback must not become the public origin");
        match err {
            OriginError::LoopbackNotPublic(ref host) => assert_eq!(host, "127.0.0.1:8088"),
            other => panic!("expected LoopbackNotPublic, got {other:?}"),
        }
        // The message has to name the fix, or the 503 is a dead end.
        assert!(err.to_string().contains("X-Forwarded-Host"));
    }

    /// One declared tenant host is not a guess. Preferring it is what keeps a
    /// co-located app working with no header and no code change.
    #[test]
    fn a_single_tenant_host_answers_for_a_loopback_caller() {
        let h = headers(&[("host", "127.0.0.1:8088")]);
        let origin = self_origin(
            &h,
            None,
            &["acme.example.com".into()],
            OriginTrust::Required,
        )
        .expect("one declared host is unambiguous");
        assert_eq!(origin, "https://acme.example.com");
    }

    /// A developer machine configures nothing, and must keep working. That case
    /// returns before the loopback check.
    #[test]
    fn loopback_still_works_when_nothing_is_configured() {
        let h = headers(&[("host", "localhost:8080")]);
        let origin =
            self_origin(&h, None, &[], OriginTrust::WarnOnce).expect("dev must keep working");
        assert_eq!(origin, "https://localhost:8080");
    }

    /// An OAuth issuer over loopback is a local client talking to a local
    /// server — the URL is used in that process, not emailed. Tightening it
    /// would break those deployments on upgrade.
    #[test]
    fn warn_once_still_allows_loopback_when_configured() {
        let h = headers(&[("host", "127.0.0.1:8088")]);
        let origin = self_origin(
            &h,
            None,
            &["acme.example.com".into(), "b.acme.test".into()],
            OriginTrust::WarnOnce,
        )
        .expect("WarnOnce keeps its long-standing behaviour");
        assert_eq!(origin, "https://127.0.0.1:8088");
    }

    #[test]
    fn loopback_is_recognised_with_and_without_a_port() {
        for host in [
            "localhost",
            "localhost:8080",
            "127.0.0.1",
            "127.0.0.1:8088",
            "::1",
            "[::1]:8080",
        ] {
            assert!(is_loopback(host), "{host} should be loopback");
        }
        for host in [
            "acme.example.com",
            "127.0.0.1.evil.test",
            "localhost.evil.test",
        ] {
            assert!(!is_loopback(host), "{host} should NOT be loopback");
        }
    }

    /// The whole point of learning: a co-located app reaches RaisinDB over
    /// loopback and cannot say which org it is, but somebody's browser already
    /// told us a moment ago.
    ///
    /// Serial with the tests below — the learned map is process-wide, so these
    /// use distinct tenant ids rather than a lock.
    #[test]
    fn a_loopback_request_uses_the_origin_the_tenant_was_last_served_on() {
        let tenant = "tenant-learn-basic";
        with_suffixes(Some(".rdb.example.com"), || {
            // A real visitor arrives through the edge.
            let served = self_origin(
                &headers(&[("host", "acme.rdb.example.com")]),
                Some(tenant),
                &[],
                OriginTrust::Required,
            )
            .expect("a suffix-matching host is allowed");
            assert_eq!(served, "https://acme.rdb.example.com");

            // Now the app asks for a magic link over loopback.
            let origin = self_origin(
                &headers(&[("host", "127.0.0.1:8088")]),
                Some(tenant),
                &[],
                OriginTrust::Required,
            )
            .expect("the learned origin answers for a loopback caller");
            assert_eq!(origin, "https://acme.rdb.example.com");
        });
    }

    /// Configuration outranks anything learned: an operator asserted it, and it
    /// survives a restart.
    #[test]
    fn a_declared_tenant_host_wins_over_a_learned_one() {
        let tenant = "tenant-learn-precedence";
        with_suffixes(Some(".rdb.example.com"), || {
            remember_origin(tenant, "https://learned.rdb.example.com");

            let origin = self_origin(
                &headers(&[("host", "127.0.0.1:8088")]),
                Some(tenant),
                &["declared.example.com".into()],
                OriginTrust::Required,
            )
            .expect("a declared host resolves");
            assert_eq!(origin, "https://declared.example.com");
        });
    }

    /// A tenant we have never served publicly still refuses, rather than
    /// borrowing another tenant's origin or guessing.
    #[test]
    fn an_unseen_tenant_still_refuses() {
        with_suffixes(Some(".rdb.example.com"), || {
            let err = self_origin(
                &headers(&[("host", "127.0.0.1:8088")]),
                Some("tenant-never-seen"),
                &[],
                OriginTrust::Required,
            )
            .expect_err("nothing learned, nothing declared");
            assert!(matches!(err, OriginError::LoopbackNotPublic(_)));
        });
    }

    /// `note_request_origin` is the teaching path, and it must be as fussy as
    /// the link-building path: a spoofed Host on ANY request would otherwise
    /// poison the origin a later magic link is built on.
    #[test]
    fn only_a_validated_public_host_is_ever_learned() {
        with_suffixes(Some(".rdb.example.com"), || {
            note_request_origin(&headers(&[("host", "evil.test")]), "tenant-teach-evil");
            assert_eq!(recall_origin("tenant-teach-evil"), None, "unlisted host");

            note_request_origin(&headers(&[("host", "127.0.0.1:8088")]), "tenant-teach-lb");
            assert_eq!(
                recall_origin("tenant-teach-lb"),
                None,
                "loopback teaches nothing"
            );

            note_request_origin(
                &headers(&[("host", "ok.rdb.example.com")]),
                "tenant-teach-ok",
            );
            assert_eq!(
                recall_origin("tenant-teach-ok").as_deref(),
                Some("https://ok.rdb.example.com")
            );
        });
    }

    /// With no allowlist there is nothing to validate against, so recording
    /// would let any Host become the tenant's origin.
    #[test]
    fn nothing_is_learned_when_no_allowlist_is_configured() {
        with_suffixes(None, || {
            note_request_origin(
                &headers(&[("host", "anything.example")]),
                "tenant-teach-none",
            );
            assert_eq!(recall_origin("tenant-teach-none"), None);
        });
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
