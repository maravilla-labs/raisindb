#![cfg(not(feature = "s3"))]
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use std::sync::Arc;
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
        let path = format!("/tmp/raisin-rocks-test-edges-{}", nanoid::nanoid!(6));
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
async fn move_into_own_descendant_is_rejected() {
    let app = app().await;
    // seed /a and /a/b
    for (id, name, path, parent) in [("a", "a", "/a", None), ("b", "b", "/a/b", Some("/a"))] {
        let mut body = serde_json::json!({"id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1});
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
    // move /a -> /a/b/a should fail
    let body = serde_json::json!({"targetPath":"/a/b/a"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/a?command=move")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn copy_into_own_descendant_is_rejected() {
    let app = app().await;
    // seed /a and /a/b
    for (id, name, path, parent) in [
        ("a", "a", "/a", None),
        ("b", "b", "/a/b", Some("/a")),
        ("dst", "dst", "/dst", None),
    ] {
        let mut body = serde_json::json!({"id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1});
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
    // copy_tree /a -> /a/b/a should fail
    let body = serde_json::json!({"targetPath":"/a/b/a"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/a?command=copy_tree")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn rename_root_and_invalid_name() {
    let app = app().await;
    // seed root child /x
    let body = serde_json::json!({"id":"x","name":"x","path":"/x","node_type":"t","properties":{},"children":[],"version":1});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/repository/ws/x")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // rename to valid new name -> /y
    let body = serde_json::json!({"newName":"y"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/x?command=rename")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    // verify
    let req = Request::builder()
        .uri("/api/repository/ws/y")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // invalid name containing slash
    let body = serde_json::json!({"newName":"bad/name"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws/y?command=rename")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deep_children_edges() {
    let app = app().await;
    // empty parent returns empty
    let req = Request::builder()
        .uri("/api/repository/ws/?level=3&flatten=true")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let flat: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(flat, serde_json::json!([]));

    // create chain depth 10 under /p
    for i in 0..10u32 {
        let name = format!("n{}", i);
        let path = if i == 0 {
            "/p/n0".to_string()
        } else {
            format!(
                "/p/n0/{}",
                (1..=i)
                    .map(|x| format!("n{}", x))
                    .collect::<Vec<_>>()
                    .join("/")
            )
        };
        let parent = if i == 0 {
            Some("/p".to_string())
        } else {
            Some(path.rsplit_once('/').unwrap().0.to_string())
        };
        let mut body = serde_json::json!({"id":format!("id{}", i),"name":name,"path":path,"node_type":"t","properties":{},"children":[],"version":1});
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p);
        }
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/repository/ws{}", path))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let _ = app.clone().oneshot(req).await.unwrap();
    }
    // request maxDepth=10 should cap at 5
    let req = Request::builder()
        .uri("/api/repository/ws/p/n0?level=10&flatten=true")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let flat: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    // flat is a map, count entries -> should be 5 (n1..n5)
    assert_eq!(flat.as_object().map(|m| m.len()).unwrap_or(0), 5);
}
