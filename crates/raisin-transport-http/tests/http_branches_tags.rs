//! Integration tests for Branch and Tag management HTTP endpoints

use axum::body::Body;
use axum::http::{Request, StatusCode};
use raisin_hlc::HLC;
use raisin_transport_http::router;
use std::sync::Arc;
use tower::ServiceExt;

#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;

/// Helper to create a test router with in-memory storage
#[cfg(not(feature = "storage-rocksdb"))]
fn test_router() -> axum::Router {
    let storage = Arc::new(InMemoryStorage::default());
    router(storage)
}

/// Helper to create a test router with RocksDB storage
#[cfg(feature = "storage-rocksdb")]
fn test_router() -> axum::Router {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let storage =
        Arc::new(RocksDBStorage::new(temp_dir.path()).expect("Failed to create RocksDBStorage"));
    router(storage)
}

/// Helper to parse JSON response
async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: axum::response::Response,
) -> T {
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read response body");
    serde_json::from_slice(&body_bytes).expect("Failed to parse JSON response")
}

// ============================================================================
// Branch Management Tests
// ============================================================================

#[tokio::test]
async fn test_create_branch() {
    let app = test_router();

    let request_body = serde_json::json!({
        "name": "develop",
        "from_revision": null,
        "created_by": "test-user",
        "protected": false
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/branches")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let branch: raisin_context::Branch = parse_json_response(response).await;
    assert_eq!(branch.name, "develop");
    assert_eq!(branch.created_by, "test-user");
    assert_eq!(branch.protected, false);
}

#[tokio::test]
async fn test_create_branch_from_revision() {
    let app = test_router();

    let request_body = serde_json::json!({
        "name": "hotfix",
        "from_revision": 42,
        "created_by": "admin",
        "protected": true
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/branches")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let branch: raisin_context::Branch = parse_json_response(response).await;
    assert_eq!(branch.name, "hotfix");
    assert_eq!(branch.created_from, Some(HLC::new(42, 0)));
    assert_eq!(branch.protected, true);
}

#[tokio::test]
async fn test_list_branches() {
    let app = test_router();

    // Create a few branches first
    for name in &["develop", "staging", "production"] {
        let request_body = serde_json::json!({
            "name": name,
            "from_revision": null,
            "created_by": "test-user",
            "protected": false
        });

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/management/repositories/default/main/branches")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    // List all branches
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/branches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let branches: Vec<raisin_context::Branch> = parse_json_response(response).await;
    assert_eq!(branches.len(), 3);

    let names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
    assert!(names.contains(&"develop".to_string()));
    assert!(names.contains(&"staging".to_string()));
    assert!(names.contains(&"production".to_string()));
}

#[tokio::test]
async fn test_get_branch() {
    let app = test_router();

    // Create a branch
    let request_body = serde_json::json!({
        "name": "feature-x",
        "from_revision": null,
        "created_by": "dev-team",
        "protected": false
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/branches")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Get the branch
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/branches/feature-x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let branch: raisin_context::Branch = parse_json_response(response).await;
    assert_eq!(branch.name, "feature-x");
    assert_eq!(branch.created_by, "dev-team");
}

#[tokio::test]
async fn test_get_nonexistent_branch() {
    let app = test_router();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/branches/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_branch() {
    let app = test_router();

    // Create a branch
    let request_body = serde_json::json!({
        "name": "temp-branch",
        "from_revision": null,
        "created_by": "test-user",
        "protected": false
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/branches")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Delete the branch
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/management/repositories/default/main/branches/temp-branch")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_get_branch_head() {
    let app = test_router();

    // Create a branch
    let request_body = serde_json::json!({
        "name": "main",
        "from_revision": null,
        "created_by": "system",
        "protected": true
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/branches")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Get branch HEAD
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/branches/main/head")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let head: u64 = parse_json_response(response).await;
    assert_eq!(head, 0); // Initial HEAD is 0
}

#[tokio::test]
async fn test_update_branch_head() {
    let app = test_router();

    // Create a branch
    let request_body = serde_json::json!({
        "name": "develop",
        "from_revision": null,
        "created_by": "system",
        "protected": false
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/branches")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Update branch HEAD
    let update_body = serde_json::json!({
        "revision": 123
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/management/repositories/default/main/branches/develop/head")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// ============================================================================
// Tag Management Tests
// ============================================================================

#[tokio::test]
async fn test_create_tag() {
    let app = test_router();

    let request_body = serde_json::json!({
        "name": "v1.0.0",
        "revision": 100,
        "created_by": "release-manager",
        "message": "First stable release",
        "protected": true
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/tags")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let tag: raisin_context::Tag = parse_json_response(response).await;
    assert_eq!(tag.name, "v1.0.0");
    assert_eq!(tag.revision, HLC::new(100, 0));
    assert_eq!(tag.created_by, "release-manager");
    assert_eq!(tag.message, Some("First stable release".to_string()));
    assert_eq!(tag.protected, true);
}

#[tokio::test]
async fn test_create_tag_minimal() {
    let app = test_router();

    let request_body = serde_json::json!({
        "name": "snapshot-2024",
        "revision": 42,
        "created_by": null,
        "message": null,
        "protected": false
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/tags")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let tag: raisin_context::Tag = parse_json_response(response).await;
    assert_eq!(tag.name, "snapshot-2024");
    assert_eq!(tag.revision, HLC::new(42, 0));
    assert_eq!(tag.created_by, "system"); // Should default to "system"
}

#[tokio::test]
async fn test_list_tags() {
    let app = test_router();

    // Create multiple tags
    for (name, revision) in &[("v1.0.0", 10), ("v1.1.0", 20), ("v2.0.0", 30)] {
        let request_body = serde_json::json!({
            "name": name,
            "revision": revision,
            "created_by": "test-user",
            "message": null,
            "protected": false
        });

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/management/repositories/default/main/tags")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    // List all tags
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/tags")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let tags: Vec<raisin_context::Tag> = parse_json_response(response).await;
    assert_eq!(tags.len(), 3);

    let names: Vec<String> = tags.iter().map(|t| t.name.clone()).collect();
    assert!(names.contains(&"v1.0.0".to_string()));
    assert!(names.contains(&"v1.1.0".to_string()));
    assert!(names.contains(&"v2.0.0".to_string()));
}

#[tokio::test]
async fn test_get_tag() {
    let app = test_router();

    // Create a tag
    let request_body = serde_json::json!({
        "name": "release-candidate",
        "revision": 99,
        "created_by": "qa-team",
        "message": "Ready for testing",
        "protected": false
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/tags")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Get the tag
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/tags/release-candidate")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let tag: raisin_context::Tag = parse_json_response(response).await;
    assert_eq!(tag.name, "release-candidate");
    assert_eq!(tag.revision, HLC::new(99, 0));
    assert_eq!(tag.created_by, "qa-team");
}

#[tokio::test]
async fn test_get_nonexistent_tag() {
    let app = test_router();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/main/tags/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_tag() {
    let app = test_router();

    // Create a tag
    let request_body = serde_json::json!({
        "name": "temp-tag",
        "revision": 5,
        "created_by": "test-user",
        "message": null,
        "protected": false
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/management/repositories/default/main/tags")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Delete the tag
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/management/repositories/default/main/tags/temp-tag")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_delete_nonexistent_tag() {
    let app = test_router();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/management/repositories/default/main/tags/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
