// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared helpers for the OAuth 2.1 authorization-server HTTP endpoints:
//! deriving the public issuer, rendering RFC-compliant error responses, and
//! mapping authorization-server errors onto HTTP status codes.

use std::sync::Once;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use raisin_auth::authserver::AuthServerError;

use crate::state::AppState;

/// Derive and validate the externally-visible issuer (base URL) for discovery
/// documents, endpoint URLs, and token audiences.
///
/// The issuer is derived per request from the `Host` (or, behind a trusted
/// proxy, `X-Forwarded-Host`), so a multi-tenant deployment serving each org at
/// its own host advertises the correct issuer per request. To stop a spoofed
/// host from minting a token with an attacker-chosen audience, the derived host
/// is validated against an allowlist:
///
/// - `RAISINDB_TRUSTED_HOST_SUFFIXES` — comma-separated host suffixes trusted for
///   every tenant (e.g. a platform's wildcard subdomain base). A suffix matches
///   its apex and any subdomain (`example.com` matches `example.com` and
///   `a.example.com`, but not `evilexample.com`).
/// - `tenant_hosts` — per-tenant exact hosts (e.g. a tenant's custom domain),
///   loaded from the tenant auth config by the caller via
///   [`load_tenant_trusted_hosts`].
/// - loopback hosts (`localhost`, `127.0.0.1`, `::1`) are always allowed.
///
/// Enforcement is opt-in: when no allowlist is configured (no suffixes and no
/// tenant hosts) the derived issuer is returned as-is — preserving single-origin
/// behavior — and a one-time warning is logged. `RAISINDB_BASE_URL`, if set, is
/// an absolute override for a single fixed origin and skips host derivation.
///
/// Returns [`AuthServerError::InvalidRequest`] when an allowlist is configured
/// and the request host is not on it. The value never has a trailing slash.
pub fn issuer_from_request(
    headers: &HeaderMap,
    tenant_hosts: &[String],
) -> Result<String, AuthServerError> {
    if let Some(base) = configured_base_url() {
        return Ok(base);
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

    let issuer = format!("{proto}://{host}").trim_end_matches('/').to_string();

    let suffixes = trusted_host_suffixes();
    if suffixes.is_empty() && tenant_hosts.is_empty() {
        warn_unconfigured_issuer_once();
        return Ok(issuer);
    }

    if host_is_trusted(host, &suffixes, tenant_hosts) {
        Ok(issuer)
    } else {
        Err(AuthServerError::InvalidRequest(format!(
            "request host `{host}` is not an allowed OAuth issuer host"
        )))
    }
}

/// Load this tenant's trusted OAuth hosts from its auth config (empty if unset).
pub(crate) async fn load_tenant_trusted_hosts(state: &AppState, tenant_id: &str) -> Vec<String> {
    state
        .storage()
        .tenant_auth_config_repository()
        .get_config(tenant_id)
        .await
        .ok()
        .flatten()
        .map(|cfg| cfg.trusted_oauth_hosts)
        .unwrap_or_default()
}

/// The configured canonical base URL, if `RAISINDB_BASE_URL` is set non-empty.
fn configured_base_url() -> Option<String> {
    std::env::var("RAISINDB_BASE_URL")
        .ok()
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
}

/// Host suffixes trusted as OAuth issuer hosts for every tenant
/// (`RAISINDB_TRUSTED_HOST_SUFFIXES`, comma-separated).
fn trusted_host_suffixes() -> Vec<String> {
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
fn host_is_trusted(host: &str, suffixes: &[String], tenant_hosts: &[String]) -> bool {
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
fn matches_host_suffix(hostname: &str, suffix: &str) -> bool {
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
fn forwarded_headers_trusted() -> bool {
    std::env::var("RAISINDB_TRUST_FORWARDED_HEADERS")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Warn once that the issuer is being derived without an allowlist.
fn warn_unconfigured_issuer_once() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "No OAuth issuer allowlist configured (RAISINDB_TRUSTED_HOST_SUFFIXES \
             unset and no per-tenant trusted hosts); the issuer and token \
             audiences are derived from the request host without validation. Set \
             RAISINDB_TRUSTED_HOST_SUFFIXES (or RAISINDB_BASE_URL for a single \
             fixed origin) in any proxied or multi-origin deployment."
        );
    });
}

/// Read a header value as a borrowed `str`, taking the first comma-separated
/// token (proxy headers may carry a list).
fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or(v).trim())
        .filter(|v| !v.is_empty())
}

/// Map an [`AuthServerError`] onto the HTTP status the OAuth specs prescribe.
pub fn status_for(err: &AuthServerError) -> StatusCode {
    match err {
        AuthServerError::InvalidClient(_) => StatusCode::UNAUTHORIZED,
        AuthServerError::ServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        AuthServerError::AccessDenied(_) => StatusCode::FORBIDDEN,
        AuthServerError::InvalidClientMetadata(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_REQUEST,
    }
}

/// Render an [`AuthServerError`] as an RFC 6749 §5.2 JSON error response.
pub fn oauth_error_response(err: &AuthServerError) -> Response {
    let status = status_for(err);
    let body = err.to_response_body();
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_host(host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", host.parse().unwrap());
        h
    }

    #[test]
    fn suffix_matches_apex_and_subdomain_only() {
        assert!(matches_host_suffix("rdb.example.cloud", "rdb.example.cloud"));
        assert!(matches_host_suffix("acme.rdb.example.cloud", ".rdb.example.cloud"));
        assert!(matches_host_suffix("acme.rdb.example.cloud", "rdb.example.cloud"));
        // Not on a label boundary — must be rejected.
        assert!(!matches_host_suffix("evilrdb.example.cloud", "rdb.example.cloud"));
        assert!(!matches_host_suffix("rdb.example.cloud.evil.com", "rdb.example.cloud"));
        assert!(!matches_host_suffix("anything", ""));
    }

    #[test]
    fn host_trust_covers_loopback_suffix_and_tenant_host() {
        let suffixes = vec![".rdb.example.cloud".to_string()];
        let tenant = vec!["app.acme.example".to_string()];

        assert!(host_is_trusted("localhost:8088", &suffixes, &tenant));
        assert!(host_is_trusted("127.0.0.1", &suffixes, &tenant));
        assert!(host_is_trusted("acme.rdb.example.cloud", &suffixes, &tenant));
        assert!(host_is_trusted("app.acme.example", &[], &tenant)); // per-tenant exact
        assert!(!host_is_trusted("evil.com", &suffixes, &tenant));
        assert!(!host_is_trusted(
            "acme.rdb.example.cloud.evil.com",
            &suffixes,
            &tenant
        ));
    }

    // These two rely on RAISINDB_* env being unset (the default for tests).
    #[test]
    fn issuer_enforces_allowlist_when_tenant_hosts_present() {
        let tenant = vec!["app.acme.example".to_string()];

        let ok = issuer_from_request(&headers_with_host("app.acme.example"), &tenant);
        assert_eq!(ok.unwrap(), "https://app.acme.example");

        let denied = issuer_from_request(&headers_with_host("evil.example"), &tenant);
        assert!(denied.is_err());
    }

    #[test]
    fn issuer_permissive_without_allowlist() {
        let issuer = issuer_from_request(&headers_with_host("anything.example"), &[]).unwrap();
        assert_eq!(issuer, "https://anything.example");
    }
}
