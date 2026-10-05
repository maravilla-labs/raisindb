//! `POST /api/tenants/{t}/embeddings/config` — the admin console's save.
//!
//! Its payload predates several settings. A setting the payload does not carry
//! must keep its stored value: writing the default instead reset a tuned
//! vector cutoff (0.78 for EmbeddingGemma) to the engine's 0.6 on every save,
//! which empties that model's whole vector leg, and would re-open anonymous
//! query embedding a tenant had denied.

#![cfg(not(feature = "s3"))]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::support::{Fixture, TENANT};

const REPO: &str = "embcfg";

async fn post_config(app: &axum::Router, body: Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tenants/{TENANT}/embeddings/config"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let text =
        String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes()).to_string();
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// The stored config, as `GET /api/tenants/{t}/embeddings/config` returns it.
async fn shown(app: &axum::Router) -> Value {
    let req = Request::builder()
        .uri(format!("/api/tenants/{TENANT}/embeddings/config"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// What the admin console sends: none of the newer settings.
fn console_payload() -> Value {
    json!({
        "enabled": false, "provider": "OpenAI", "model": "text-embedding-3-small",
        "dimensions": 8, "include_name": true, "include_path": true,
    })
}

#[tokio::test]
async fn a_console_save_keeps_the_cutoff_prefixes_and_anonymous_policy() {
    let fx = Fixture::new("embedding-config-keep", REPO, "main", &["demo"], &[]).await;
    let app = fx.app;

    let mut full = console_payload();
    full["default_max_distance"] = json!(0.78);
    full["query_prefix"] = json!("task: search result | query: ");
    full["anonymous_query_embeddings"] = json!("deny");
    post_config(&app, full).await;
    let before = shown(&app).await;
    assert!(
        (before["default_max_distance"].as_f64().unwrap() - 0.78).abs() < 1e-6,
        "{before}"
    );
    assert_eq!(before["anonymous_query_embeddings"], "deny");

    // the console saves again, without the newer fields
    post_config(&app, console_payload()).await;
    let after = shown(&app).await;
    assert!(
        (after["default_max_distance"].as_f64().unwrap() - 0.78).abs() < 1e-6,
        "the cutoff survived the save: {after}"
    );
    assert_eq!(after["query_prefix"], "task: search result | query: ");
    assert_eq!(after["anonymous_query_embeddings"], "deny");

    // a payload that DOES carry the cutoff still sets it
    let mut explicit = console_payload();
    explicit["default_max_distance"] = json!(0.65);
    post_config(&app, explicit).await;
    assert!((shown(&app).await["default_max_distance"].as_f64().unwrap() - 0.65).abs() < 1e-6);
}
