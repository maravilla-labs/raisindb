#![cfg(not(feature = "s3"))]
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
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
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
            "default",
            "default",
            "main",
            test_node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

/// Test creating a root node using POST with parent path semantics
#[tokio::test]
async fn create_root_node_legacy_api() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-create-root";
            let _ = std::fs::remove_dir_all(path);
            let store = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let store = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
    };

    // First create workspace
    let ws_body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/main/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // POST to root path "/" creates node at root
    // URL path = parent location, server auto-generates child path
    let node_body = serde_json::json!({
        "name": "about",
        "node_type": "page",
        "properties": {}
    });
    eprintln!(
        "Request JSON: {}",
        serde_json::to_string_pretty(&node_body).unwrap()
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    if status != StatusCode::CREATED && status != StatusCode::OK {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let error_text = String::from_utf8_lossy(&bytes);
        eprintln!("ERROR RESPONSE: {} - {}", status, error_text);
        panic!("Request failed with status {}", status);
    }
    assert!(status == StatusCode::CREATED || status == StatusCode::OK);

    // Verify the created node
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(node.name, "about");
    assert_eq!(node.path, "/about");
    assert_eq!(node.node_type, "page");
    assert_eq!(node.parent, Some("/".to_string())); // Parent is root, displayed as "/"

    // Verify we can GET the node by path
    let req = Request::builder()
        .uri("/api/repository/main/main/head/demo/about")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );
}

/// Test creating a child node under a parent
#[tokio::test]
async fn create_child_node_legacy_api() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-create-child";
            let _ = std::fs::remove_dir_all(path);
            let store = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let store = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
    };

    // Create workspace
    let ws_body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/main/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Create parent node
    let parent_body = serde_json::json!({
        "name": "company",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&parent_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );

    // Create child under /company
    // POST to /company creates child at /company/{name}
    let child_body = serde_json::json!({
        "name": "team",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/company")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&child_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );

    // Verify the created child node
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(node.name, "team");
    assert_eq!(node.path, "/company/team");
    assert_eq!(node.node_type, "page");
    assert_eq!(node.parent, Some("company".to_string())); // Parent NAME, not PATH

    // Verify we can GET the node by path
    let req = Request::builder()
        .uri("/api/repository/main/main/head/demo/company/team")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );
}

/// Test deep node creation (auto-creates missing parent folders)
#[tokio::test]
async fn create_node_with_deep_parent_creation() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-deep-create";
            let _ = std::fs::remove_dir_all(path);
            let store = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*store, "page").await;
            create_test_node_type(&*store, "raisin:Folder").await;
            raisin_transport_http::router(store)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let store = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*store, "page").await;
            create_test_node_type(&*store, "raisin:Folder").await;
            raisin_transport_http::router(store)
        }
    };

    // Create workspace
    let ws_body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/main/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Create node with auto-created parent folders using ?deep=true
    // Server should auto-create /projects, /projects/2024, /projects/2024/q1 as raisin:Folder nodes
    let node_body = serde_json::json!({
        "name": "report",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/projects/2024/q1?deep=true")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );

    // Verify the created node
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(node.name, "report");
    assert_eq!(node.path, "/projects/2024/q1/report");
    assert_eq!(node.node_type, "page");
    assert_eq!(node.parent, Some("q1".to_string())); // Parent NAME, not PATH

    // Verify parent folders were created as raisin:Folder nodes
    let req = Request::builder()
        .uri("/api/repository/main/main/head/demo/projects")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let folder: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(folder.node_type, "raisin:Folder");

    let req = Request::builder()
        .uri("/api/repository/main/main/head/demo/projects/2024")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let folder: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(folder.node_type, "raisin:Folder");
}

/// Test that duplicate names are rejected
#[tokio::test]
async fn reject_duplicate_node_names() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-duplicate";
            let _ = std::fs::remove_dir_all(path);
            let store = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let store = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
    };

    // Create workspace
    let ws_body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/main/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Create first node
    let node_body = serde_json::json!({
        "name": "about",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );

    // Try to create duplicate node with same name under same parent
    let node_body = serde_json::json!({
        "name": "about",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // Should return BAD_REQUEST because name already exists
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Test that parent must exist (without deep flag)
#[tokio::test]
async fn reject_missing_parent_without_deep() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-missing-parent";
            let _ = std::fs::remove_dir_all(path);
            let store = RocksDBStorage::new(path).unwrap();
            raisin_transport_http::router(Arc::new(store))
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            raisin_transport_http::router(Arc::new(InMemoryStorage::default()))
        }
    };

    // Create workspace
    let ws_body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/main/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Try to create node under non-existent parent (without deep flag)
    let node_body = serde_json::json!({
        "name": "child",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/nonexistent/parent")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // Should return NOT_FOUND because parent doesn't exist
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Test name sanitization
#[tokio::test]
async fn sanitize_node_names() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-sanitize";
            let _ = std::fs::remove_dir_all(path);
            let store = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let store = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*store, "page").await;
            raisin_transport_http::router(store)
        }
    };

    // Create workspace
    let ws_body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/main/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Create node with name that needs sanitization
    let node_body = serde_json::json!({
        "name": "My Page!!  With Spaces",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/demo/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "Expected 200 OK or 201 CREATED, got {}",
        status
    );

    // Verify the node name was sanitized
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    // Name should be sanitized (exact sanitization depends on implementation)
    assert_ne!(node.name, "My Page!!  With Spaces");
    // Path should use sanitized name
    assert_eq!(node.path, format!("/{}", node.name));
}
