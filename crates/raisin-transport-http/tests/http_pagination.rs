#![cfg(not(feature = "s3"))]
use std::sync::Arc;

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_models::nodes::types::NodeType;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;
use raisin_transport_http as http;

#[derive(serde::Deserialize)]
struct Page<T> {
    items: Vec<T>,
    page: PageMeta,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageMeta {
    total: usize,
    limit: usize,
    offset: usize,
    next_offset: Option<usize>,
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

async fn app() -> axum::Router {
    #[cfg(feature = "storage-rocksdb")]
    {
        let path = format!("/tmp/raisin-rocks-test-pagination-{}", nanoid::nanoid!(8));
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
async fn pagination_under_query_parent_limit_1() {
    let app = app().await;
    // seed /p with 3 children in stable order
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("a", "a", "/p/a", Some("/p")),
        ("b", "b", "/p/b", Some("/p")),
        ("c", "c", "/p/c", Some("/p")),
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

    // POST /:ws/query with parent and limit=1, then paginate
    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        let req_body = serde_json::json!({"parent":"/p","limit":1,"offset":offset});
        let req = Request::builder()
            .method("POST")
            .uri("/ws/query")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let page: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
        collected.extend(page.items.iter().map(|n| n.name.clone()));
        if let Some(next) = page.page.next_offset {
            offset = next;
        } else {
            break;
        }
    }
    assert_eq!(collected, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn workspaces_list_pagination_ordered_by_name() {
    let app = app().await;
    // create 3 workspaces out of order
    for name in ["c", "a", "b"] {
        let body = serde_json::json!({"name": name, "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/workspaces/{}", name))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // GET /workspaces?limit=2&offset=0
    let req = Request::builder()
        .uri("/workspaces?limit=2&offset=0")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let page: Page<raisin_models::workspace::Workspace> = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<_> = page.items.iter().map(|w| w.name.clone()).collect();
    assert_eq!(names, vec!["a", "b"]);
    assert_eq!(page.page.next_offset, Some(2));

    // Next page offset=2
    let req = Request::builder()
        .uri("/workspaces?limit=2&offset=2")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let page: Page<raisin_models::workspace::Workspace> = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<_> = page.items.iter().map(|w| w.name.clone()).collect();
    assert_eq!(names, vec!["c"]);
    assert_eq!(page.page.next_offset, None);
}
