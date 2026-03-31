#![cfg(not(feature = "s3"))]
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt; // for collect
use tower::ServiceExt; // for oneshot

use raisin_models::nodes::types::NodeType;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;
use raisin_transport_http as http;
// no extra imports needed for multipart test

#[derive(serde::Deserialize)]
struct Page<T> {
    items: Vec<T>,
}

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

#[tokio::test]
async fn health_is_ok() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-health";
            let _ = std::fs::remove_dir_all(path);
            let store = RocksDBStorage::new(path).unwrap();
            // ensure a fresh DB path per test run if you want isolation; using a unique path per test name here
            raisin_transport_http::router(Arc::new(store))
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            raisin_transport_http::router(Arc::new(InMemoryStorage::default()))
        }
    };
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn workspace_put_and_get() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-ws";
            let _ = std::fs::remove_dir_all(path);
            let store = RocksDBStorage::new(path).unwrap();
            raisin_transport_http::router(Arc::new(store))
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            raisin_transport_http::router(Arc::new(InMemoryStorage::default()))
        }
    };

    // PUT workspace
    let body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/workspaces/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET workspace
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/workspaces/demo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let ws: raisin_models::workspace::Workspace = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ws.name, "demo");
}

#[tokio::test]
async fn node_put_get_delete() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-node";
            let _ = std::fs::remove_dir_all(path);
            let storage = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*storage, "t").await;
            raisin_transport_http::router(storage)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let storage = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*storage, "t").await;
            raisin_transport_http::router(storage)
        }
    };

    // PUT node under ws "demo" using path-based repo API
    let body = serde_json::json!({
        "id": "n1",
        "name": "node1",
        "path": "/node1",
        "node_type": "t",
        "properties": {},
        "children": [],
        "version": 1
    });
    let req = Request::builder()
        .method("PUT")
        .uri(
            "/api/repository/demo/".to_string()
                + body["path"].as_str().unwrap().trim_start_matches('/'),
        )
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET node by path
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/repository/demo/node1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // DELETE node by path
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/repository/demo/node1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn query_endpoints() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-query";
            let _ = std::fs::remove_dir_all(path);
            let storage = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*storage, "alpha").await;
            create_test_node_type(&*storage, "beta").await;
            raisin_transport_http::router(storage)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let storage = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*storage, "alpha").await;
            create_test_node_type(&*storage, "beta").await;
            raisin_transport_http::router(storage)
        }
    };

    // seed nodes
    for (id, name, path, parent, t) in [
        ("a", "A", "/a", None, "alpha"),
        ("b", "B", "/a/b", Some("/a"), "beta"),
        ("c", "C", "/a/c", Some("/a"), "beta"),
    ] {
        let mut body = serde_json::json!({
            "id": id,
            "name": name,
            "path": path,
            "node_type": t,
            "properties": {},
            "children": [],
            "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/demo".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // query by type
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"nodeType":"beta"})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 2);

    // query by parent
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"parent":"/a"})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 2);

    // combined filters (parent + type)
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"parent":"/a","nodeType":"beta"})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 2);

    // pagination (limit=1)
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"parent":"/a","limit":1})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 1);

    // query by path
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"path":"/a"})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 1);

    // bad request when neither filter provided
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        error: String,
        message: String,
    }
    let e: ErrorBody = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(e.error, "BadRequest");
}

#[tokio::test]
async fn repo_multipart_upload_sets_resource() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-upload";
            let _ = std::fs::remove_dir_all(path);
            let storage = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*storage, "t").await;
            raisin_transport_http::router(storage)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let storage = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*storage, "t").await;
            raisin_transport_http::router(storage)
        }
    };

    // First create the node
    let node_body = serde_json::json!({
        "id": "test-node",
        "name": "node",
        "path": "/path/to/node",
        "node_type": "t",
        "properties": {},
        "children": [],
        "version": 1
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/api/repository/ws1/path/to/node")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Create a multipart body with one file field "file"
    let boundary = "XBOUNDARY";
    let mut buf: Vec<u8> = Vec::new();
    use std::io::Write;
    write!(buf, "--{}\r\n", boundary).unwrap();
    write!(
        buf,
        "Content-Disposition: form-data; name=\"file\"; filename=\"hello.txt\"\r\n"
    )
    .unwrap();
    write!(buf, "Content-Type: text/plain\r\n\r\n").unwrap();
    write!(buf, "hello world\n").unwrap();
    write!(buf, "\r\n--{}--\r\n", boundary).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/ws1/path/to/node")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(buf))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(v.get("storedKey").is_some());

    // Fetch the node and verify properties.file is set
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/repository/ws1/path/to/node")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let n: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    let file_prop = n.properties.get("file").cloned();
    match file_prop {
        Some(raisin_models::nodes::properties::PropertyValue::Resource(r)) => {
            assert_eq!(r.name.as_deref(), Some("hello.txt"));
            assert_eq!(r.mime_type.as_deref(), Some("text/plain"));
        }
        other => panic!("expected Resource property, got {:?}", other),
    }
}

#[tokio::test]
async fn query_dsl_endpoint() {
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-dsl";
            let _ = std::fs::remove_dir_all(path);
            let storage = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*storage, "alpha").await;
            create_test_node_type(&*storage, "beta").await;
            raisin_transport_http::router(storage)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let storage = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*storage, "alpha").await;
            create_test_node_type(&*storage, "beta").await;
            raisin_transport_http::router(storage)
        }
    };

    // seed nodes
    for (id, name, path, parent, t) in [
        ("a", "A", "/a", None, "alpha"),
        ("b", "B", "/a/b", Some("/a"), "beta"),
        ("c", "C", "/a/c", Some("/a"), "beta"),
    ] {
        let mut body = serde_json::json!({
            "id": id,
            "name": name,
            "path": path,
            "node_type": t,
            "properties": {},
            "children": [],
            "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/demo".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // DSL: and [ { nodeType in ["beta"] }, { parent eq "/a" } ] with limit 1
    let dsl = serde_json::json!({
        "and": [
            { "nodeType": { "in": ["beta"] } },
            { "parent": { "eq": "/a" } }
        ],
        "order_by": { "path": "asc" },
        "limit": 1,
        "offset": 0
    });
    let req = Request::builder()
        .method("POST")
        .uri("/demo/query/dsl")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&dsl).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 1);
}
