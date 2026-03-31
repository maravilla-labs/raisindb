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

async fn app() -> axum::Router {
    #[cfg(feature = "storage-rocksdb")]
    {
        let path = format!("/tmp/raisin-rocks-test-collisions-{}", nanoid::nanoid!(8));
        let _ = std::fs::remove_dir_all(&path);
        let storage = Arc::new(RocksDBStorage::new(&path).unwrap());

        // Create test NodeType
        let test_node_type = NodeType {
            id: Some("t".to_string()),
            strict: Some(false),
            name: "t".to_string(),
            extends: None,
            mixins: vec![],
            overrides: None,
            description: Some("Test NodeType".to_string()),
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

        return raisin_transport_http::router(storage);
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let storage = Arc::new(InMemoryStorage::default());

        // Create test NodeType
        let test_node_type = NodeType {
            id: Some("t".to_string()),
            strict: Some(false),
            name: "t".to_string(),
            extends: None,
            mixins: vec![],
            overrides: None,
            description: Some("Test NodeType".to_string()),
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

        raisin_transport_http::router(storage)
    }
}

#[tokio::test]
async fn rename_conflict_returns_500() {
    let app = app().await;
    // seed /p with a and b
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("a", "a", "/p/a", Some("/p")),
        ("b", "b", "/p/b", Some("/p")),
    ] {
        let mut body = serde_json::json!({
            "id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // repo command rename: /p/a -> new_name "b" (conflict)
    let body = serde_json::json!({"newName":"b"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/p/a?command=rename")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn move_conflict_returns_500() {
    let app = app().await;
    // seed /q with a and b
    for (id, name, path, parent) in [
        ("q", "q", "/q", None),
        ("a", "a", "/q/a", Some("/q")),
        ("b", "b", "/q/b", Some("/q")),
    ] {
        let mut body = serde_json::json!({
            "id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // repo command move: move /q/a to /q/b (conflict)
    let body = serde_json::json!({"targetPath":"/q/b"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/q/a?command=move")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn move_into_parent_with_same_child_name_conflicts() {
    let app = app().await;
    // seed /src with a; /dst with a
    for (id, name, path, parent) in [
        ("src", "src", "/src", None),
        ("a1", "a", "/src/a", Some("/src")),
        ("dst", "dst", "/dst", None),
        ("a2", "a", "/dst/a", Some("/dst")),
    ] {
        let mut body = serde_json::json!({
            "id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // move /src/a -> /dst/a (same name conflict)
    let body = serde_json::json!({"targetPath":"/dst/a"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/src/a?command=move")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
