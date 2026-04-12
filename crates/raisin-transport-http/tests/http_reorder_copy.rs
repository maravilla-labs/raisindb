#![cfg(not(feature = "s3"))]
use std::sync::Arc;

use axum::{body::Body, http::Request};
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
            BranchScope::new("default", "default", "main"),
            test_node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

#[cfg_attr(not(feature = "storage-rocksdb"), allow(unused_variables))]
async fn app_with_test_nodetype(path_suffix: &str) -> axum::Router {
    #[cfg(feature = "storage-rocksdb")]
    {
        let path = format!("/tmp/raisin-rocks-test-{}", path_suffix);
        let _ = std::fs::remove_dir_all(&path);
        let storage = Arc::new(RocksDBStorage::new(&path).unwrap());
        create_test_node_type(&*storage, "t").await;
        let app = raisin_transport_http::router(storage);

        // Create workspace
        let ws_body = serde_json::json!({"name": "ws", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
        let req = Request::builder()
            .method("PUT")
            .uri("/api/workspaces/main/ws")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
            .unwrap();
        let _ = app.clone().oneshot(req).await.unwrap();

        app
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let storage = Arc::new(InMemoryStorage::default());
        create_test_node_type(&*storage, "t").await;
        raisin_transport_http::router(storage)
    }
}

#[tokio::test]
async fn nodes_reorder_endpoint_moves_child_to_position() {
    let app = app_with_test_nodetype("reorder-pos").await;

    // seed parent and children using POST to parent path
    for (id, name, parent_path) in [
        ("p", "a", "/"),
        ("b", "b", "/a"),
        ("c", "c", "/a"),
        ("d", "d", "/a"),
    ] {
        let body = serde_json::json!({
            "id": id,
            "name": name,
            "node_type": "t",
            "properties": {}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/repository/main/main/head/ws".to_string() + parent_path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        if !status.is_success() {
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let error_text = String::from_utf8_lossy(&bytes);
            eprintln!(
                "ERROR creating node {} at {}: {} - {}",
                name, parent_path, status, error_text
            );
            panic!("Failed to create node");
        }
    }

    // Verify nodes were created
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/a")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parent: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    eprintln!("Parent node /a: {:?}", parent);
    eprintln!("Children: {:?}", parent["children"]);

    // reorder: move 'd' before 'b'
    let body = serde_json::json!({"targetPath":"/a/b","movePosition":"before"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/a/d/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    if !status.is_success() {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let error_text = String::from_utf8_lossy(&bytes);
        eprintln!("ERROR reorder command: {} - {}", status, error_text);
    }
    assert_eq!(status, axum::http::StatusCode::OK);

    // verify order: d, b, c
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/a/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<String> = v.into_iter().map(|n| n.name).collect();
    assert_eq!(names, vec!["d", "b", "c"]);
}

#[tokio::test]
async fn repo_command_reorder_before_and_after() {
    let app = app_with_test_nodetype("reorder-cmd").await;

    // seed /p with x, y, z
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("x", "x", "/p/x", Some("/p")),
        ("y", "y", "/p/y", Some("/p")),
        ("z", "z", "/p/z", Some("/p")),
    ] {
        let mut body = serde_json::json!({
            "id": id,
            "name": name,
            "path": path,
            "node_type": "t",
            "properties": {},
            "children": [],
            "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/main/main/head/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // move z before y
    let body = serde_json::json!({"targetPath":"/p/y","movePosition":"before"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/p/z/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // order should be x, z, y
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/p/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let names: Vec<String> = serde_json::from_slice::<Vec<raisin_models::nodes::Node>>(
        &resp.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect();
    assert_eq!(names, vec!["x", "z", "y"]);

    // move x after z -> z, x, y
    let body = serde_json::json!({"targetPath":"/p/z","movePosition":"after"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/p/x/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/p/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let names: Vec<String> = serde_json::from_slice::<Vec<raisin_models::nodes::Node>>(
        &resp.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect();
    assert_eq!(names, vec!["z", "x", "y"]);
}

#[tokio::test]
async fn reorder_position_beyond_len_appends_and_noop_self_moves() {
    let app = app_with_test_nodetype("reorder-edge").await;

    // seed /p with x, y
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("x", "x", "/p/x", Some("/p")),
        ("y", "y", "/p/y", Some("/p")),
    ] {
        let mut body = serde_json::json!({ "id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1 });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/main/main/head/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // reorder beyond len -> append: move x after y
    let body = serde_json::json!({"targetPath":"/p/y","movePosition":"after"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/p/x/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    // order should be y, x
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/p/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let names: Vec<String> = serde_json::from_slice::<Vec<raisin_models::nodes::Node>>(
        &resp.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect();
    assert_eq!(names, vec!["y", "x"]);

    // move_before self: no-op
    let body = serde_json::json!({"targetPath":"/p/x","movePosition":"before"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/p/x/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let _ = app.clone().oneshot(req).await.unwrap();
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/p/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let names2: Vec<String> = serde_json::from_slice::<Vec<raisin_models::nodes::Node>>(
        &resp.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect();
    assert_eq!(names2, vec!["y", "x"]);

    // move_after self: no-op
    let body = serde_json::json!({"targetPath":"/p/x","movePosition":"after"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/p/x/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let _ = app.clone().oneshot(req).await.unwrap();
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/p/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let names3: Vec<String> = serde_json::from_slice::<Vec<raisin_models::nodes::Node>>(
        &resp.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect();
    assert_eq!(names3, vec!["y", "x"]);
}

#[tokio::test]
async fn reorder_before_with_missing_target_returns_404_and_no_change() {
    let app = app_with_test_nodetype("reorder-missing").await;

    // seed /p with x, y
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("x", "x", "/p/x", Some("/p")),
        ("y", "y", "/p/y", Some("/p")),
    ] {
        let mut body = serde_json::json!({ "id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1 });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/main/main/head/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // move x before non-existent sibling -> should return 404 and keep order as x, y
    let body = serde_json::json!({"targetPath":"/p/zzz","movePosition":"before"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/p/x/raisin:cmd/reorder")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);

    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/p/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let names: Vec<String> = serde_json::from_slice::<Vec<raisin_models::nodes::Node>>(
        &resp.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect();
    assert_eq!(names, vec!["x", "y"]);
}

#[tokio::test]
async fn copy_tree_to_same_path_conflicts() {
    let app = app_with_test_nodetype("copy-conflict").await;

    // seed /dst/a
    for (id, name, path, parent) in [
        ("dst", "dst", "/dst", None),
        ("a", "a", "/dst/a", Some("/dst")),
    ] {
        let mut body = serde_json::json!({
            "id": id, "name": name, "path": path, "node_type": "t", "properties": {}, "children": [], "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/main/main/head/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // attempt copy_tree /dst/a -> /dst/a (same path) should fail (500)
    let body = serde_json::json!({"targetPath":"/dst/a"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/dst/a/raisin:cmd/copy_tree")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn repo_command_copy_tree_copies_descendants() {
    let app = app_with_test_nodetype("copy-tree").await;

    // seed /src/a with /src/a/b and /src/a/c and create /dst parent
    for (id, name, path, parent) in [
        ("src", "src", "/src", None),
        ("a", "a", "/src/a", Some("/src")),
        ("b", "b", "/src/a/b", Some("/src/a")),
        ("c", "c", "/src/a/c", Some("/src/a")),
        ("dst", "dst", "/dst", None),
    ] {
        let mut body = serde_json::json!({
            "id": id,
            "name": name,
            "path": path,
            "node_type": "t",
            "properties": {},
            "children": [],
            "version": 1
        });
        if let Some(p) = parent {
            body["parent"] = serde_json::Value::String(p.to_string());
        }
        let req = Request::builder()
            .method("PUT")
            .uri("/api/repository/main/main/head/ws".to_string() + path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // copy_tree /src/a -> /dst/a (target_path is the parent, so "/dst")
    let body = serde_json::json!({"targetPath":"/dst"});
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/main/main/head/ws/src/a/raisin:cmd/copy_tree")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // verify /dst/a exists
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/dst/a")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // list /dst/a/ children
    let req = Request::builder()
        .uri("/api/repository/main/main/head/ws/dst/a/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let children: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    let mut names: Vec<String> = children.into_iter().map(|n| n.name).collect();
    names.sort();
    assert_eq!(names, vec!["b", "c"]);
}
