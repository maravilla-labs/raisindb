// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared helpers for the OAuth 2.1 authorization-server HTTP endpoints:
//! deriving the public issuer, rendering RFC-compliant error responses, and
//! mapping authorization-server errors onto HTTP status codes.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use raisin_auth::authserver::AuthServerError;

use crate::origin::{canonical_origin, self_origin, OriginError, OriginTrust};
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
    // Delegates: `self_origin` IS this logic, lifted out so the magic-link
    // email, the MCP resource id and this issuer cannot drift into three
    // different answers to one question. `WarnOnce` preserves the behaviour
    // this endpoint has always had for a deployment that configures nothing —
    // tightening it would break those on upgrade and is a separate decision.
    self_origin(headers, None, tenant_hosts, OriginTrust::WarnOnce).map_err(|e| match e {
        OriginError::UntrustedHost(host) => AuthServerError::InvalidRequest(format!(
            "request host `{host}` is not an allowed OAuth issuer host"
        )),
        OriginError::NotConfigured => {
            AuthServerError::InvalidRequest(OriginError::NotConfigured.to_string())
        }
        // Unreachable under `WarnOnce`, which allows a loopback issuer rather
        // than refusing one. Spelled out rather than caught by a wildcard so
        // that adding a refusal to `self_origin` breaks the build here instead
        // of silently reclassifying it as a bad request.
        e @ OriginError::LoopbackNotPublic(_) => AuthServerError::InvalidRequest(e.to_string()),
    })
}

/// The canonical origin of an absolute URL (`None` if it does not parse or has
/// no host), for comparing a client-supplied resource indicator against the
/// issuer this deployment actually serves.
pub(crate) fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    parsed.host_str()?;
    Some(canonical_origin(&parsed.origin().ascii_serialization()))
}

/// The canonical RFC 8707 resource URL identifying one MCP endpoint.
///
/// This is the single definition of an MCP server's audience. It is what the
/// protected-resource metadata advertises, what `/authorize` binds into the
/// authorization code (and hence the token's `aud`), and what the MCP endpoint
/// verifies the presented token against — all three must agree byte for byte.
pub(crate) fn mcp_resource_url(issuer: &str, repo: &str, branch: &str, slug: &str) -> String {
    format!("{}/mcp/{repo}/{branch}/{slug}", canonical_origin(issuer))
}

/// The RFC 9728 protected-resource metadata URL for an MCP endpoint — the value
/// a `401` challenge points at so a client can discover the authorization server.
pub(crate) fn mcp_resource_metadata_url(
    issuer: &str,
    repo: &str,
    branch: &str,
    slug: &str,
) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}",
        canonical_origin(issuer)
    )
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
    // The host-matching primitives moved to `crate::origin` so the issuer, the
    // MCP resource id and the magic-link email share one implementation. Their
    // tests stayed here rather than being duplicated there.
    use crate::origin::{host_is_trusted, matches_host_suffix};

    fn headers_with_host(host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", host.parse().unwrap());
        h
    }

    #[test]
    fn suffix_matches_apex_and_subdomain_only() {
        assert!(matches_host_suffix(
            "rdb.example.cloud",
            "rdb.example.cloud"
        ));
        assert!(matches_host_suffix(
            "acme.rdb.example.cloud",
            ".rdb.example.cloud"
        ));
        assert!(matches_host_suffix(
            "acme.rdb.example.cloud",
            "rdb.example.cloud"
        ));
        // Not on a label boundary — must be rejected.
        assert!(!matches_host_suffix(
            "evilrdb.example.cloud",
            "rdb.example.cloud"
        ));
        assert!(!matches_host_suffix(
            "rdb.example.cloud.evil.com",
            "rdb.example.cloud"
        ));
        assert!(!matches_host_suffix("anything", ""));
    }

    #[test]
    fn host_trust_covers_loopback_suffix_and_tenant_host() {
        let suffixes = vec![".rdb.example.cloud".to_string()];
        let tenant = vec!["app.acme.example".to_string()];

        assert!(host_is_trusted("localhost:8088", &suffixes, &tenant));
        assert!(host_is_trusted("127.0.0.1", &suffixes, &tenant));
        assert!(host_is_trusted(
            "acme.rdb.example.cloud",
            &suffixes,
            &tenant
        ));
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
    fn canonical_origin_normalizes_case_default_port_and_slash() {
        // Every spelling below is the same origin and must canonicalize alike.
        for spelling in [
            "https://RDB.Example.Test",
            "https://rdb.example.test/",
            "https://rdb.example.test:443",
            "HTTPS://rdb.example.test:443/",
        ] {
            assert_eq!(
                canonical_origin(spelling),
                "https://rdb.example.test",
                "failed for {spelling}"
            );
        }

        assert_eq!(canonical_origin("http://localhost:80"), "http://localhost");
        // A non-default port is significant and must be kept.
        assert_eq!(
            canonical_origin("http://localhost:8088/"),
            "http://localhost:8088"
        );
        // A default port for the *other* scheme is not a default port here.
        assert_eq!(
            canonical_origin("https://example.com:80"),
            "https://example.com:80"
        );
    }

    #[test]
    fn mcp_resource_url_is_stable_across_issuer_spellings() {
        let canonical = "https://db.example.com/mcp/studio/main/studio";
        for issuer in [
            "https://db.example.com",
            "https://db.example.com/",
            "https://DB.Example.com:443",
        ] {
            assert_eq!(
                mcp_resource_url(issuer, "studio", "main", "studio"),
                canonical,
                "failed for {issuer}"
            );
        }

        // Path segments are case-sensitive and must NOT be folded.
        assert_eq!(
            mcp_resource_url("https://db.example.com", "Studio", "main", "Studio"),
            "https://db.example.com/mcp/Studio/main/Studio"
        );
    }

    #[test]
    fn metadata_url_matches_the_rfc9728_well_known_path() {
        assert_eq!(
            mcp_resource_metadata_url("https://db.example.com/", "repo", "main", "srv"),
            "https://db.example.com/.well-known/oauth-protected-resource/mcp/repo/main/srv"
        );
    }

    #[test]
    fn issuer_permissive_without_allowlist() {
        let issuer = issuer_from_request(&headers_with_host("anything.example"), &[]).unwrap();
        assert_eq!(issuer, "https://anything.example");
    }
}
