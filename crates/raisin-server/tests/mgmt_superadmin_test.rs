//! Integration test: superadmin (/management/admin/*) endpoints are token-gated
//! and only exist when RAISIN_SUPERADMIN_TOKEN is set.
//!
//! Run with: `cargo test --package raisin-server --test mgmt_superadmin_test -- --ignored --nocapture`

mod helpers;

use helpers::multi_node::{ServerConfig, ServerHandle};
use reqwest::Client;

/// Helper: hit the admin route with an optional bearer token + tenant header.
async fn admin_get(
    base_url: &str,
    path: &str,
    bearer: Option<&str>,
) -> Result<reqwest::Response, String> {
    let client = Client::new();
    let url = format!("{}{}", base_url, path);
    let mut req = client.get(&url).header("x-tenant-id", "default");
    if let Some(t) = bearer {
        req = req.bearer_auth(t);
    }
    req.send().await.map_err(|e| e.to_string())
}

#[tokio::test]
#[ignore] // requires running server
async fn admin_routes_404_when_env_unset() {
    // No RAISIN_SUPERADMIN_TOKEN — admin subtree must not be mounted.
    // Start server WITHOUT the env var.
    let config = ServerConfig::new(8210);
    let server = ServerHandle::start(config)
        .await
        .expect("server start failed");

    let r = admin_get(&server.base_url, "/management/admin/jobs", None)
        .await
        .expect("request failed");
    assert_eq!(
        r.status().as_u16(),
        404,
        "expected 404 when env var unset, got {}",
        r.status()
    );
}

#[tokio::test]
#[ignore] // requires running server
async fn admin_routes_401_on_bad_token() {
    // The test runner sets RAISIN_SUPERADMIN_TOKEN for this case, then spawns
    // the server. Skip if no token is set.
    let token = std::env::var("RAISIN_SUPERADMIN_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("set RAISIN_SUPERADMIN_TOKEN to enable this test; skipping");
        return;
    }

    let config = ServerConfig::new(8211);
    let server = ServerHandle::start(config)
        .await
        .expect("server start failed");

    let r = admin_get(
        &server.base_url,
        "/management/admin/jobs",
        Some("not-the-token"),
    )
    .await
    .expect("request failed");
    assert_eq!(
        r.status().as_u16(),
        401,
        "expected 401 on bad bearer, got {}",
        r.status()
    );
}

#[tokio::test]
#[ignore] // requires running server
async fn admin_routes_ok_with_correct_token() {
    let token = std::env::var("RAISIN_SUPERADMIN_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("set RAISIN_SUPERADMIN_TOKEN to enable this test; skipping");
        return;
    }

    let config = ServerConfig::new(8212);
    let server = ServerHandle::start(config)
        .await
        .expect("server start failed");

    let r = admin_get(&server.base_url, "/management/admin/jobs", Some(&token))
        .await
        .expect("request failed");
    assert!(
        r.status().is_success(),
        "expected 2xx with correct bearer, got {}",
        r.status()
    );
}
