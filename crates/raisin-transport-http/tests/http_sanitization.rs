#![cfg(not(feature = "s3"))]
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
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
            "test",
            "test",
            "main",
            test_node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

async fn app() -> axum::Router {
    #[cfg(feature = "storage-rocksdb")]
    {
        let path = format!("/tmp/raisin-rocks-test-sanitize-{}", nanoid::nanoid!(6));
        let _ = std::fs::remove_dir_all(&path);
        let storage = Arc::new(RocksDBStorage::new(&path).unwrap());

        // Create test NodeType
        create_test_node_type(&*storage, "t").await;

        return raisin_transport_http::router(storage);
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let storage = Arc::new(InMemoryStorage::default());

        // Create test NodeType
        create_test_node_type(&*storage, "t").await;

        raisin_transport_http::router(storage)
    }
}

#[tokio::test]
async fn put_by_path_rejects_whitespace_leaf() {
    let app = app().await;
    // leaf is whitespace only -> sanitized name becomes empty -> 400
    let node = serde_json::json!({
        "id":"n1", "name":"ignored", "path":"/   ", "node_type":"t", "properties":{}, "children":[], "version":1
    });
    // percent-encode the space-containing path
    let req = Request::builder()
        .method("PUT")
        .uri("/api/repository/ws/%20%20%20")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn put_by_path_sanitizes_simple() {
    let app = app().await;
    // name will be sanitized: " Hello World " -> "hello-world"
    let node = serde_json::json!({
        "id":"n2", "name":"any", "path":"/ Hello World ", "node_type":"t", "properties":{}, "children":[], "version":1
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/api/repository/ws/%20Hello%20World%20")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // fetch via repo API normalized path
    let req = Request::builder()
        .uri("/api/repository/ws/hello-world")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn rename_sanitizes_and_rejects_bad() {
    let app = app().await;
    // seed /x
    let node = serde_json::json!({
        "id":"x","name":"x","path":"/x","node_type":"t","properties":{},"children":[],"version":1
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/api/repository/ws/x")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // rename to " Hello World " -> ok
    let body = serde_json::json!({"newName":" Hello World "});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/x?command=rename")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // rename to contains '/' -> 400
    let body = serde_json::json!({"newName":"bad/name"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/hello-world?command=rename")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
