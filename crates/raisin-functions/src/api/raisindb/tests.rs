// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for RaisinFunctionApi

use super::RaisinFunctionApi;

#[test]
fn test_glob_match() {
    // Test simple wildcard
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/*",
        "https://api.example.com/users"
    ));

    // Test double wildcard
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/**",
        "https://api.example.com/users/123/profile"
    ));

    // Test exact match
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/users",
        "https://api.example.com/users"
    ));

    // Test no match
    assert!(!RaisinFunctionApi::glob_match(
        "https://api.example.com/*",
        "https://api.other.com/users"
    ));
}

// ========== IMAP network-policy + credential-redaction tests ==========

use crate::api::callbacks::RaisinFunctionApiCallbacks;
use crate::types::{ExecutionContext, NetworkPolicy};

const SECRET_PW: &str = "SUPER_SECRET_APP_PASSWORD";

fn imap_conn(host: &str) -> serde_json::Value {
    serde_json::json!({
        "host": host,
        "port": 993,
        "tls": true,
        "username": "user@example.org",
        "password": SECRET_PW,
    })
}

fn api_with_allowed(allowed: &[&str]) -> RaisinFunctionApi {
    let policy = NetworkPolicy {
        http_enabled: true,
        allowed_urls: allowed.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    RaisinFunctionApi::new(ctx, policy, RaisinFunctionApiCallbacks::default())
}

/// A host that is not on the allow-list must be refused with a policy error
/// BEFORE any socket is opened — so the failure is a permission error, never a
/// connection/timeout error.
#[tokio::test]
async fn imap_disallowed_host_refused_before_connect() {
    let api = api_with_allowed(&["imaps://allowed.example.org:993"]);
    let conn = imap_conn("blocked.example.org");

    let err = api
        .impl_imap_fetch_since(conn, 0, None)
        .await
        .expect_err("disallowed host must be refused");

    let msg = err.to_string();
    // Policy denial, not a protocol/timeout error → proves the check ran first.
    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {msg}"
    );
    assert!(msg.contains("policy_denied"), "unexpected message: {msg}");
    // The password must never leak into an error surface.
    assert!(
        !msg.contains(SECRET_PW),
        "error message leaked the password: {msg}"
    );
}

/// Same guard on the other two entry points.
#[tokio::test]
async fn imap_disallowed_host_refused_all_ops() {
    let api = api_with_allowed(&["imaps://allowed.example.org:993"]);

    for r in [
        api.impl_imap_list_mailboxes(imap_conn("blocked.example.org"))
            .await,
        api.impl_imap_fetch_message(imap_conn("blocked.example.org"), 1, None)
            .await,
    ] {
        let err = r.expect_err("disallowed host must be refused");
        assert!(matches!(err, raisin_error::Error::PermissionDenied(_)));
        assert!(!err.to_string().contains(SECRET_PW));
    }
}

// ========== Crypto (verifyJwt) network-policy tests ==========

/// A JWKS URL whose host is not on the allow-list must be refused with a policy
/// error BEFORE any socket is opened — so the failure is a permission error,
/// never a fetch/timeout error.
#[tokio::test]
async fn verify_jwt_disallowed_jwks_refused_before_fetch() {
    let api = api_with_allowed(&["https://allowed.example.org/**"]);
    let opts = serde_json::json!({ "jwks_url": "https://blocked.example.org/jwks" });

    let err = api
        .impl_crypto_verify_jwt("header.payload.sig", opts)
        .await
        .expect_err("disallowed JWKS host must be refused");

    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {err}"
    );
    assert!(
        err.to_string().contains("policy_denied"),
        "unexpected message: {err}"
    );
}

/// A missing `jwks_url` is a soft failure (`{ valid:false }`), not a hard error,
/// and never touches the network.
#[tokio::test]
async fn verify_jwt_missing_jwks_url_is_soft_invalid() {
    let api = api_with_allowed(&[]);
    let out = api
        .impl_crypto_verify_jwt("t", serde_json::json!({}))
        .await
        .expect("missing jwks_url must not be a hard error");
    assert_eq!(out["valid"], serde_json::json!(false));
}

/// The connection descriptor must redact the password from any Debug render.
#[test]
fn imap_conn_debug_redacts_password() {
    use crate::runtime::imap::ImapConn;
    let conn = ImapConn::from_value(&imap_conn("imap.example.org")).unwrap();
    let dbg = format!("{conn:?}");
    assert!(!dbg.contains(SECRET_PW), "Debug leaked password: {dbg}");
    assert!(dbg.contains("<redacted>"), "Debug should mark redaction");
    assert!(dbg.contains("imap.example.org"));
}
