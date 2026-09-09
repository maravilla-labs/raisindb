// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Where the browser is allowed to land after an OIDC login.
//!
//! The callback finishes by handing the browser an access token and a refresh
//! token. Whoever controls the landing URL therefore receives those tokens, so
//! the caller-supplied `redirect_uri` is an allow-list decision, not a
//! convenience. Accepting it unchecked would turn `/auth/oidc/{provider}` into
//! a token exfiltration endpoint: a link with someone else's host in the query
//! string logs the victim in and posts their session to the attacker.
//!
//! Two things are allowed. A path on this server, which cannot leave the
//! origin. And an absolute URL whose origin appears in the tenant's
//! `cors_allowed_origins`, which is the list an administrator already maintains
//! to say which front ends belong to this tenant.

use axum::http::StatusCode;
use raisin_models::auth::TenantAuthConfig;

use crate::error::ApiError;

/// Resolve the caller's requested landing URL against the tenant allow-list.
///
/// `None` in means the caller wants the token pair as JSON rather than a
/// redirect, and `None` comes back out.
pub fn resolve_app_redirect(
    config: &TenantAuthConfig,
    requested: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(raw) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };

    // A relative path stays on this origin. `//evil.example` is excluded
    // because a browser reads it as a protocol-relative URL to another host,
    // which is the classic way past a naive "starts with /" check.
    if raw.starts_with('/') && !raw.starts_with("//") {
        return Ok(Some(raw.to_string()));
    }

    let parsed = url::Url::parse(raw).map_err(|_| refused(raw))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        // `javascript:` and `data:` would execute in the browser with the
        // tokens appended to them.
        return Err(refused(raw));
    }

    let origin = origin_of(&parsed);
    let permitted = config
        .cors_allowed_origins
        .iter()
        .filter_map(|allowed| url::Url::parse(allowed).ok())
        .any(|allowed| origin_of(&allowed) == origin);

    if permitted {
        Ok(Some(raw.to_string()))
    } else {
        Err(refused(raw))
    }
}

/// Scheme, host and port, which is what "same site" means for this purpose.
///
/// Compared as a normalised triple rather than by string, so that
/// `https://app.example.com` and `https://app.example.com:443` are recognised
/// as one origin and a trailing slash does not matter.
fn origin_of(url: &url::Url) -> (String, String, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default(),
    )
}

fn refused(raw: &str) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "REDIRECT_NOT_ALLOWED",
        format!(
            "redirect_uri '{raw}' is not permitted; add its origin to the tenant's \
             cors_allowed_origins, or omit redirect_uri to receive the tokens as JSON"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(origins: &[&str]) -> TenantAuthConfig {
        let mut c = TenantAuthConfig::new("acme".to_string());
        c.cors_allowed_origins = origins.iter().map(|s| s.to_string()).collect();
        c
    }

    #[test]
    fn no_redirect_means_json() {
        assert_eq!(resolve_app_redirect(&config(&[]), None).unwrap(), None);
        assert_eq!(
            resolve_app_redirect(&config(&[]), Some("  ")).unwrap(),
            None
        );
    }

    #[test]
    fn a_relative_path_is_allowed_with_no_list_at_all() {
        let got = resolve_app_redirect(&config(&[]), Some("/after-login")).unwrap();
        assert_eq!(got.as_deref(), Some("/after-login"));
    }

    /// The bug a "starts with /" check walks into: a browser treats this as
    /// `https://evil.example`, so it must not count as a local path.
    #[test]
    fn a_protocol_relative_url_is_not_a_local_path() {
        assert!(resolve_app_redirect(&config(&[]), Some("//evil.example/x")).is_err());
    }

    #[test]
    fn an_allow_listed_origin_is_permitted() {
        let c = config(&["https://app.example.com"]);
        assert!(resolve_app_redirect(&c, Some("https://app.example.com/landing?a=1")).is_ok());
    }

    /// The whole point. An attacker's URL must not receive the token pair,
    /// even though the login itself would have succeeded.
    #[test]
    fn an_unlisted_origin_is_refused() {
        let c = config(&["https://app.example.com"]);
        let err = resolve_app_redirect(&c, Some("https://evil.example/steal")).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    /// A host that merely starts with an allowed one must not pass. This is
    /// what comparing origins rather than string prefixes buys.
    #[test]
    fn a_lookalike_host_is_refused() {
        let c = config(&["https://app.example.com"]);
        assert!(resolve_app_redirect(&c, Some("https://app.example.com.evil.test/x")).is_err());
    }

    #[test]
    fn a_different_scheme_or_port_is_a_different_origin() {
        let c = config(&["https://app.example.com"]);
        assert!(resolve_app_redirect(&c, Some("http://app.example.com/x")).is_err());
        assert!(resolve_app_redirect(&c, Some("https://app.example.com:8443/x")).is_err());
    }

    /// Explicit and default ports are the same origin.
    #[test]
    fn an_explicit_default_port_matches() {
        let c = config(&["https://app.example.com:443"]);
        assert!(resolve_app_redirect(&c, Some("https://app.example.com/x")).is_ok());
    }

    #[test]
    fn a_javascript_url_is_refused() {
        let c = config(&["https://app.example.com"]);
        assert!(resolve_app_redirect(&c, Some("javascript:alert(1)")).is_err());
        assert!(resolve_app_redirect(&c, Some("data:text/html,x")).is_err());
    }

    /// A dev front end on localhost is normal and works once listed.
    #[test]
    fn a_listed_localhost_origin_works() {
        let c = config(&["http://localhost:5173"]);
        assert!(resolve_app_redirect(&c, Some("http://localhost:5173/after-login")).is_ok());
    }
}
