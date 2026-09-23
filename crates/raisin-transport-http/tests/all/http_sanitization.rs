#![cfg(not(feature = "s3"))]
//! Name sanitization on create and rename over the current repository API.
//! The fixture creates repository `test` with workspace `ws` and sends as the
//! operator — see `support`.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

/// The HEAD route of workspace `ws` in repository `test`.
const WS: &str = "/api/repository/test/main/head/ws";

async fn app() -> axum::Router {
    crate::support::Fixture::new("sanitize", "test", "main", &["ws"], &["t"])
        .await
        .app
}

#[tokio::test]
async fn create_rejects_whitespace_name() {
    let app = app().await;
    // A node is created by POST on its parent, and its path segment is its
    // sanitized NAME. A whitespace-only name sanitizes to nothing -> 400.
    let (status, text) = crate::support::create_at(&app, WS, "a", "a", "/a", "t").await;
    assert!(status.is_success(), "seeding /a: {status} {text}");
    let node = serde_json::json!({
        "id":"n1", "name":"   ", "node_type":"t", "properties":{}
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/a"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_sanitizes_simple_name() {
    let app = app().await;
    // A child's path segment is its sanitized name: " Hello World " -> "hello-world"
    let (status, text) = crate::support::create_at(&app, WS, "a", "a", "/a", "t").await;
    assert!(status.is_success(), "seeding /a: {status} {text}");
    let node = serde_json::json!({
        "id":"n2", "name":" Hello World ", "node_type":"t", "properties":{}
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/a"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // fetch via repo API normalized path
    let req = Request::builder()
        .uri(format!("{WS}/a/hello-world"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn rename_sanitizes_and_rejects_bad() {
    let app = app().await;
    // seed /x
    let (status, text) = crate::support::create_at(&app, WS, "x", "x", "/x", "t").await;
    assert!(status.is_success(), "seeding /x: {status} {text}");

    // rename to " Hello World " -> ok
    let body = serde_json::json!({"newName":" Hello World "});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/x?command=rename"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // rename to contains '/' -> 400
    let body = serde_json::json!({"newName":"bad/name"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/hello-world?command=rename"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
