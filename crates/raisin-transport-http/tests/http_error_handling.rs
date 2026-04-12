#![cfg(not(feature = "s3"))]
//! Error handling tests for the HTTP transport layer
//!
//! This test suite validates that the HTTP API returns correct error codes:
//! - 404 for non-existent resources
//! - 400 for malformed requests
//! - 413 for payload too large
//! - Empty arrays only when resource exists but has no children

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_models::nodes::types::NodeType;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository, Storage};
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;
use raisin_transport_http as http;

async fn create_test_node_type<S: Storage>(storage: &S, name: &str) {
    let test_node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: vec![],
        overrides: None,
        description: Some(format!("Test NodeType: {}", name)),
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec![],
        required_nodes: vec![],
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(false),
        indexable: None,
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        compound_indexes: None,
        is_mixin: None,
        previous_version: None,
    };
    storage
        .node_types()
        .put(
            BranchScope::new("test", "test", "main"),
            test_node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

async fn setup_app() -> axum::Router {
    #[cfg(feature = "storage-rocksdb")]
    {
        let path = format!("/tmp/raisin-rocks-test-errors-{}", nanoid::nanoid!(8));
        let _ = std::fs::remove_dir_all(&path);
        let storage = Arc::new(RocksDBStorage::new(&path).unwrap());

        // Create test NodeTypes
        create_test_node_type(&*storage, "page").await;
        create_test_node_type(&*storage, "folder").await;

        raisin_transport_http::router(storage)
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let storage = Arc::new(InMemoryStorage::default());

        // Create test NodeTypes
        create_test_node_type(&*storage, "page").await;
        create_test_node_type(&*storage, "folder").await;

        raisin_transport_http::router(storage)
    }
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
        .uri("/workspaces/test")
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
        .uri("/api/repository/test/nonexistent")
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
        .uri("/api/repository/test/nonexistent/")
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
        .uri("/api/repository/test/nonexistent/?level=2")
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
        .uri("/api/repository/test/nonexistent/?level=2&flatten=true")
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
        .uri("/api/repository/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // List children - should return empty array, NOT 404
    let req = Request::builder()
        .uri("/api/repository/test/parent/")
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
        .uri("/api/repository/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Create multipart form with small file (< 11MB)
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let small_content = "Small file content that's under 11MB";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"small.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, small_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/docs?inline=true")
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
        .uri("/api/repository/test/docs")
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
        .uri("/api/repository/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Create multipart form with large file (> 11MB)
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let large_content = "x".repeat(12 * 1024 * 1024); // 12MB
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"large.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, large_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/docs?inline=true")
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
        .uri("/api/repository/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

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
        .uri("/api/repository/test/docs?inline=true")
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
        .uri("/api/repository/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Upload first file inline
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let first_content = "First version of file";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--{}--\r\n",
        boundary, first_content, boundary
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/test/docs?inline=true")
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
        .uri("/api/repository/test/docs?inline=true&override=true")
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
        .uri("/api/repository/test/docs")
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
        .uri("/api/repository/test/")
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
        .uri("/api/repository/test/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Get deep children (nested) - should return empty map
    let req = Request::builder()
        .uri("/api/repository/test/parent/?level=2")
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
        .uri("/api/repository/test/parent/?level=2&flatten=true")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Deep children (flat) of existing node should return 200"
    );

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let children: std::collections::HashMap<String, raisin_models::nodes::Node> =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        children.len(),
        0,
        "Should return empty map for childless parent"
    );
}
