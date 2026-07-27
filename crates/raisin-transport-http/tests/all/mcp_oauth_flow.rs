#![cfg(all(not(feature = "s3"), feature = "storage-rocksdb"))]

//! OAuth 2.1 / RFC 8707 resource-indicator handling for MCP endpoints.
//!
//! These guard the failure that made a fully-successful OAuth flow produce an
//! unusable token: the audience was minted from the *client's* spelling of the
//! resource URL and verified against the *server's* reconstruction of it, so a
//! trailing slash or an explicit `:443` silently degraded every MCP request to
//! anonymous. Everything here asserts the two spellings now converge.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use raisin_rocksdb::RocksDBStorage;
use tower::ServiceExt;

fn app(test_name: &str) -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = RocksDBStorage::new(dir.path().join(test_name).to_str().unwrap()).unwrap();
    (raisin_transport_http::router(Arc::new(store)), dir)
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// Register a public client and return its `client_id`.
async fn register_client(app: &axum::Router, host: &str) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("host", host)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "redirect_uris": ["http://127.0.0.1:9000/cb"],
                "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code"],
                "response_types": ["code"],
                "client_name": "test-client",
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "client registration should succeed"
    );
    json_body(response).await["client_id"]
        .as_str()
        .expect("client_id in registration response")
        .to_string()
}

async fn authorize_get(
    app: &axum::Router,
    host: &str,
    client_id: &str,
    resource: &str,
) -> axum::response::Response {
    let uri = format!(
        "/authorize?response_type=code&client_id={client_id}\
         &redirect_uri={redirect}&code_challenge={challenge}&code_challenge_method=S256\
         &resource={resource}",
        redirect = urlencoding::encode("http://127.0.0.1:9000/cb"),
        // A syntactically valid S256 challenge; these tests never redeem a code.
        challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        resource = urlencoding::encode(resource),
    );
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("host", host)
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

/// The discovery document must advertise the *canonical* resource URL, since
/// that string becomes the token's audience and is later reconstructed by the
/// MCP endpoint. Odd-but-equivalent `Host` spellings must not change it.
#[tokio::test]
async fn protected_resource_metadata_advertises_a_canonical_resource() {
    let (app, _dir) = app("metadata-canonical");

    for host in ["db.example.com", "DB.Example.COM", "db.example.com:443"] {
        let request = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-protected-resource/mcp/studio/main/studio")
            .header("host", host)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "host {host}");

        let body = json_body(response).await;
        assert_eq!(
            body["resource"].as_str(),
            Some("https://db.example.com/mcp/studio/main/studio"),
            "resource must be canonical for host {host}"
        );
    }
}

/// The regression guard. A client that asks for `…/studio/` (or `:443`, or a
/// mixed-case host) must still get through `/authorize`: the server normalizes
/// the indicator rather than binding the token to a string it will never
/// reconstruct.
#[tokio::test]
async fn authorize_accepts_equivalent_resource_spellings() {
    let (app, _dir) = app("authorize-spellings");
    let client_id = register_client(&app, "db.example.com").await;

    for resource in [
        "https://db.example.com/mcp/studio/main/studio",
        "https://db.example.com/mcp/studio/main/studio/",
        "https://db.example.com:443/mcp/studio/main/studio",
        "https://DB.Example.COM/mcp/studio/main/studio",
    ] {
        let response = authorize_get(&app, "db.example.com", &client_id, resource).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "resource spelling `{resource}` should be accepted and canonicalized"
        );

        // The consent form carries the bound resource in a hidden field, and
        // that value is what `POST /authorize` binds into the code and hence the
        // token's `aud`. Asserting on it is asserting on the future audience.
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&bytes);
        assert!(
            html.contains(
                r#"name="resource" value="https://db.example.com/mcp/studio/main/studio""#
            ),
            "`{resource}` must be bound in canonical form, got:\n{html}"
        );
    }
}

/// A client may not name an audience on some other origin — otherwise it could
/// have the server mint a token bound to a resource it does not host.
#[tokio::test]
async fn authorize_rejects_a_cross_origin_resource() {
    let (app, _dir) = app("authorize-cross-origin");
    let client_id = register_client(&app, "db.example.com").await;

    let response = authorize_get(
        &app,
        "db.example.com",
        &client_id,
        "https://evil.example/mcp/studio/main/studio",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"].as_str(), Some("invalid_target"));
}

/// A resource indicator that is not an MCP endpoint URL has no `(repo, branch,
/// slug)` to bind to and must be refused as a target, not silently accepted.
#[tokio::test]
async fn authorize_rejects_a_non_mcp_resource() {
    let (app, _dir) = app("authorize-non-mcp");
    let client_id = register_client(&app, "db.example.com").await;

    let response = authorize_get(
        &app,
        "db.example.com",
        &client_id,
        "https://db.example.com/api/studio/main",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"].as_str(), Some("invalid_target"));
}
