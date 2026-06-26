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

/// Derive the externally-visible base URL (issuer) for discovery documents,
/// endpoint URLs, and token audiences.
///
/// The issuer is treated as **configuration, not request data**: token audiences
/// derive from it, so a spoofed `Host` / `X-Forwarded-*` header must not be able
/// to rewrite the audience a resource server expects.
///
/// Precedence:
/// 1. `RAISINDB_BASE_URL` — authoritative when set. A deployment behind a reverse
///    proxy (TLS terminator / tenant router) MUST set it so issuer and audience
///    stay canonical regardless of inbound headers.
/// 2. `X-Forwarded-Proto` + `X-Forwarded-Host` — honoured ONLY when
///    `RAISINDB_TRUST_FORWARDED_HEADERS` is truthy, i.e. the operator guarantees
///    these are set by a trusted proxy and cannot be client-spoofed. Off by
///    default, since these headers are attacker-controlled on a directly
///    reachable server.
/// 3. The request `Host` header (scheme `https`), defaulting to `localhost`.
///
/// When neither (1) nor a trusted proxy is configured, a one-time warning is
/// logged: a request-host-derived issuer is unsafe for a proxied or multi-origin
/// deployment.
///
/// The returned value never has a trailing slash.
pub fn issuer_from_request(headers: &HeaderMap) -> String {
    if let Some(base) = configured_base_url() {
        return base;
    }

    warn_unconfigured_issuer_once();

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

    format!("{proto}://{host}").trim_end_matches('/').to_string()
}

/// The configured canonical base URL, if `RAISINDB_BASE_URL` is set non-empty.
fn configured_base_url() -> Option<String> {
    std::env::var("RAISINDB_BASE_URL")
        .ok()
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
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

/// Warn once that the issuer is being derived from request headers.
fn warn_unconfigured_issuer_once() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "RAISINDB_BASE_URL is not set; the OAuth issuer and token audiences \
             are derived from request headers. Set RAISINDB_BASE_URL to the \
             canonical external base URL in any proxied or multi-origin \
             deployment so token audiences cannot be influenced by spoofed \
             request headers."
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
