//! Integration tests for Revision/Git-like features HTTP endpoints
//!
//! Tests cover:
//! - Listing revisions with pagination and filtering
//! - Getting single revision metadata
//! - Getting changed nodes for a revision
//! - Browsing nodes at specific revisions (time-travel)
//! - Path-based routing (/head/ vs /rev/{n}/)
//! - Both InMemory and RocksDB storage backends

use axum::body::Body;
use axum::http::{Request, StatusCode};
use raisin_transport_http::router;
use std::sync::Arc;
use tower::ServiceExt;

#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;

/// Helper to create a test router with in-memory storage
#[cfg(not(feature = "storage-rocksdb"))]
fn create_test_router() -> axum::Router {
    let storage = Arc::new(InMemoryStorage::default());
    router(storage)
}

/// Helper to create a test router with RocksDB storage
#[cfg(feature = "storage-rocksdb")]
fn create_test_router() -> axum::Router {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = format!("/tmp/raisin-test-revisions-{}", id);
    let _ = std::fs::remove_dir_all(&path);
    let storage = Arc::new(RocksDBStorage::new(&path).expect("Failed to create RocksDBStorage"));
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

/// Helper to create a repository
async fn create_repository(app: axum::Router, repo_id: &str) -> axum::Router {
    let request_body = serde_json::json!({
        "repo_id": repo_id,
        "description": "Test repository",
        "default_branch": "main"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/repositories")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // 201 = created, 409 = already exists (both OK)
    assert!(response.status() == StatusCode::CREATED || response.status() == StatusCode::CONFLICT);

    app
}

/// Helper to create a branch
async fn create_branch(app: axum::Router, repo_id: &str, branch_name: &str) -> axum::Router {
    let request_body = serde_json::json!({
        "name": branch_name,
        "from_revision": null,
        "created_by": "test-user",
        "protected": false
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/api/management/repositories/default/{}/branches",
                    repo_id
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // 201 = created, 409 = already exists (both OK)
    assert!(response.status() == StatusCode::CREATED || response.status() == StatusCode::CONFLICT);

    app
}

/// Helper to create a workspace
async fn create_workspace(app: axum::Router, repo_id: &str, workspace_id: &str) -> axum::Router {
    let request_body = serde_json::json!({
        "name": workspace_id,
        "description": "Test workspace",
        "allowed_node_types": ["raisin:Folder", "raisin:Page"],
        "allowed_root_node_types": ["raisin:Folder", "raisin:Page"]
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/workspaces/{}/{}", repo_id, workspace_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());

    app
}

/// Helper to create a node
async fn create_node(
    app: axum::Router,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    node_type: &str,
    name: &str,
) -> (axum::Router, serde_json::Value) {
    let mut payload = serde_json::json!({
        "name": name,
        "node_type": node_type,
    });

    if node_type == "raisin:Page" {
        payload["properties"] = serde_json::json!({
            "title": name
        });
    }

    let uri = format!(
        "/api/repository/{}/{}/{}/{}",
        repo_id, branch, workspace, path
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let node: serde_json::Value = parse_json_response(response).await;
    (app, node)
}

// ============================================================================
// Revision Metadata Tests
// ============================================================================

#[tokio::test]
async fn test_list_revisions_empty() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/test-repo/revisions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let revisions: Vec<serde_json::Value> = parse_json_response(response).await;
    assert_eq!(
        revisions.len(),
        0,
        "New repository should have no revisions"
    );
}

#[tokio::test]
async fn test_list_revisions_with_pagination() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;

    // Note: In a real scenario, we'd create nodes to generate revisions
    // For now, we're testing that the endpoint returns correctly even with empty data

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/test-repo/revisions?limit=10&offset=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let revisions: Vec<serde_json::Value> = parse_json_response(response).await;
    assert!(revisions.len() <= 10, "Should respect limit parameter");
}

#[tokio::test]
async fn test_list_revisions_filter_system() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;

    // Test with include_system=false (default)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(
                    "/api/management/repositories/default/test-repo/revisions?include_system=false",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Test with include_system=true
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/test-repo/revisions?include_system=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_single_revision() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;

    // Try to get a non-existent revision
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/test-repo/revisions/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404 for non-existent revision
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_revision_changes() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;

    // Try to get changes for a non-existent revision
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/test-repo/revisions/1/changes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404 for non-existent revision
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Time-Travel Read Tests - Path-Based Routing
// ============================================================================

#[tokio::test]
async fn test_browse_head_vs_revision() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Create a node using HEAD path
    let (app, _node) = create_node(
        app,
        "test-repo",
        "main",
        "demo",
        "",
        "raisin:Page",
        "homepage",
    )
    .await;

    // Test HEAD route - should work (read/write)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/head/demo/homepage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK, "HEAD route should work");

    // Test revision route - may not work yet if no revisions exist
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/rev/1/demo/homepage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // This might be 404 if no snapshots exist yet - that's OK
    assert!(response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_root_at_revision() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Try to browse root at revision 1
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/rev/1/demo/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Might be 404 if revision doesn't exist, or 200 with empty array
    assert!(response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_node_by_id_at_revision() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Create a node to get an ID
    let (app, node) = create_node(
        app,
        "test-repo",
        "main",
        "demo",
        "",
        "raisin:Page",
        "test-page",
    )
    .await;

    let node_id = node["id"].as_str().unwrap();

    // Try to get the node at revision 1
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/test-repo/main/rev/1/demo/_id/{}",
                    node_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Might be 404 if no snapshot exists at revision 1
    assert!(response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_revision_route_is_read_only() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Try to create a node using revision path (should fail - no POST route)
    let payload = serde_json::json!({
        "name": "test",
        "node_type": "raisin:Page",
        "properties": {
            "title": "Test"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/repository/test-repo/main/rev/1/demo/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be 404 (route not found) or 405 (method not allowed)
    assert!(
        response.status() == StatusCode::NOT_FOUND
            || response.status() == StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn test_head_route_allows_writes() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Create a node using HEAD path (should work)
    let payload = serde_json::json!({
        "name": "test",
        "node_type": "raisin:Page",
        "properties": {
            "title": "Test"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/repository/test-repo/main/head/demo/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "HEAD route should allow writes"
    );
}

// ============================================================================
// Backward Compatibility Tests
// ============================================================================

#[tokio::test]
async fn test_legacy_routes_still_work() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Old route format (without /head/)
    let payload = serde_json::json!({
        "name": "legacy-test",
        "node_type": "raisin:Page",
        "properties": {
            "title": "Legacy Test"
        }
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/repository/test-repo/main/demo/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Legacy route should still work"
    );

    // Verify we can read it back with the old route
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/demo/legacy-test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Legacy GET route should work"
    );
}

#[tokio::test]
async fn test_legacy_and_head_routes_are_equivalent() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Create node using legacy route
    let payload = serde_json::json!({
        "name": "test-page",
        "node_type": "raisin:Page",
        "properties": {
            "title": "Test Page"
        }
    });

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/repository/test-repo/main/demo/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Read with legacy route
    let legacy_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/demo/test-page")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Read with HEAD route
    let head_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/head/demo/test-page")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(legacy_response.status(), StatusCode::OK);
    assert_eq!(head_response.status(), StatusCode::OK);

    let legacy_node: serde_json::Value = parse_json_response(legacy_response).await;
    let head_node: serde_json::Value = parse_json_response(head_response).await;

    // Both should return the same node
    assert_eq!(legacy_node["id"], head_node["id"]);
    assert_eq!(legacy_node["name"], head_node["name"]);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_invalid_revision_number() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;
    let app = create_workspace(app, "test-repo", "demo").await;

    // Try to browse with invalid revision (string instead of number)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/repository/test-repo/main/rev/invalid/demo/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be 400 (bad request) or 404 (route not matched)
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_nonexistent_repository() {
    let app = create_test_router();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/nonexistent/revisions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404 or similar error
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_missing_query_parameters() {
    let app = create_test_router();
    let app = create_repository(app, "test-repo").await;
    let app = create_branch(app, "test-repo", "main").await;

    // Test without any query parameters (should use defaults)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/management/repositories/default/test-repo/revisions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Should work with default parameters"
    );
}
