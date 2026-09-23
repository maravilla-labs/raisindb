#![cfg(not(feature = "s3"))]
//! Error handling tests for the HTTP transport layer
//!
//! This test suite validates that the HTTP API returns correct error codes:
//! - 404 for non-existent resources
//! - 400 for malformed requests
//! - 413 for payload too large
//! - Empty arrays only when resource exists but has no children

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Repository `test` with the `page` and `folder` node types, sent as the
/// operator — see `support`. Each test creates workspace `test` itself.
async fn setup_app() -> axum::Router {
    crate::support::Fixture::new("errors", "test", "main", &[], &["page", "folder"])
        .await
        .app
}

async fn create_workspace(app: &axum::Router) {
    let ws_body = serde_json::json!({
        "name": "test",
        "description": "Test workspace",
        "allowed_node_types": ["page", "folder"],
        "allowed_root_node_types": ["page", "folder"],
        "depends_on": []
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/test/test")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_get_nonexistent_node_returns_404() {
    let app = setup_app().await;
    create_workspace(&app).await;

    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/nonexistent")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_list_children_of_nonexistent_parent_returns_404() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Try to list children of non-existent parent
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/nonexistent/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Listing children of non-existent parent should return 404"
    );
}

#[tokio::test]
async fn test_deep_children_of_nonexistent_parent_returns_404() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Try to get deep children of non-existent parent (nested)
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/nonexistent/?level=2")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Deep children (nested) of non-existent parent should return 404"
    );

    // Try to get deep children of non-existent parent (flattened)
    let app = setup_app().await;
    create_workspace(&app).await;

    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/nonexistent/?level=2&flatten=true")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Deep children (flat) of non-existent parent should return 404"
    );
}

#[tokio::test]
async fn test_list_children_of_childless_parent_returns_empty_array() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Create a node with no children
    let node_body = serde_json::json!({
        "name": "parent",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // List children - should return empty array, NOT 404
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/parent/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Listing children of existing but childless node should return 200"
    );

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let children: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        children.len(),
        0,
        "Should return empty array for childless parent"
    );
}

#[tokio::test]
async fn test_inline_upload_with_size_limit() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Create a parent node
    let node_body = serde_json::json!({
        "name": "docs",
        "node_type": "folder",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Create multipart form with small file (< 11MB)
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let small_content = "Small file content that's under 11MB";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"small.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, small_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/docs?inline=true")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Small inline upload should succeed"
    );

    // Verify file was stored inline
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/docs")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();

    // Check that file property exists and contains string content
    if let Some(raisin_models::nodes::properties::PropertyValue::String(content)) =
        node.properties.get("file")
    {
        assert_eq!(content, small_content, "Inline file content should match");
    } else {
        panic!("Expected inline file to be stored as String property");
    }
}

#[tokio::test]
async fn test_inline_upload_size_exceeds_limit() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Create a parent node
    let node_body = serde_json::json!({
        "name": "docs",
        "node_type": "folder",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Create multipart form with large file (> 11MB)
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let large_content = "x".repeat(12 * 1024 * 1024); // 12MB
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"large.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, large_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/docs?inline=true")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "Large inline upload should fail with 413"
    );
}

#[tokio::test]
async fn test_inline_upload_non_utf8_fails() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Create a parent node
    let node_body = serde_json::json!({
        "name": "docs",
        "node_type": "folder",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Create multipart form with invalid UTF-8 bytes
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let invalid_utf8 = vec![0xFF, 0xFE, 0xFD]; // Invalid UTF-8 sequence

    let mut body = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"binary.bin\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(&invalid_utf8);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/docs?inline=true")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Non-UTF8 inline upload should fail with 400"
    );
}

#[tokio::test]
async fn test_override_replaces_existing_file() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Create a parent node
    let node_body = serde_json::json!({
        "name": "docs",
        "node_type": "folder",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Upload first file inline
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let first_content = "First version of file";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, first_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/docs?inline=true")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Override with second file
    let second_content = "Second version of file - replaced!";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, second_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/docs?inline=true&override=true")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Override should succeed");

    // Verify file was replaced
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/docs")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();

    // Check that file property contains new content
    if let Some(raisin_models::nodes::properties::PropertyValue::String(content)) =
        node.properties.get("file")
    {
        assert_eq!(content, second_content, "File content should be replaced");
        assert_ne!(content, first_content, "Old content should be gone");
    } else {
        panic!("Expected file to be stored as String property");
    }
}

#[tokio::test]
async fn test_root_listing_returns_empty_for_new_workspace() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // List root - should return empty array since workspace exists but has no nodes
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Root listing should return 200"
    );

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let children: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        children.len(),
        0,
        "Should return empty array for new workspace"
    );
}

#[tokio::test]
async fn test_deep_children_of_childless_parent_returns_empty() {
    let app = setup_app().await;
    create_workspace(&app).await;

    // Create a node with no children
    let node_body = serde_json::json!({
        "name": "parent",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/main/head/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Get deep children (nested) - should return empty map. `format=map`
    // selects the keyed map; the default is the array form.
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/parent/?level=2&format=map")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Deep children of existing node should return 200"
    );

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let children: std::collections::HashMap<String, raisin_models::nodes::DeepNode> =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        children.len(),
        0,
        "Should return empty map for childless parent"
    );

    // Get deep children (flat) - should return empty map
    let req = Request::builder()
        .uri("/api/repository/test/main/head/test/parent/?level=2&flatten=true&format=map")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Deep children (flat) of existing node should return 200"
    );

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    // The flattened form is a list of nodes.
    let children: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        children.len(),
        0,
        "Should return an empty list for childless parent"
    );
}
