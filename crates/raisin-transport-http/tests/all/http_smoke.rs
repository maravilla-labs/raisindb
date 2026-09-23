#![cfg(not(feature = "s3"))]
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt; // for collect
use tower::ServiceExt; // for oneshot

/// The HEAD routes of workspaces `demo` and `ws1` in repository `test`. The
/// fixtures create the repository and send as the operator — see `support`.
const DEMO: &str = "/api/repository/test/main/head/demo";
const WS1: &str = "/api/repository/test/main/head/ws1";

#[derive(serde::Deserialize)]
struct Page<T> {
    items: Vec<T>,
}

#[tokio::test]
async fn health_is_ok() {
    let app = crate::support::Fixture::new("smoke-health", "test", "main", &["demo", "ws1"], &[])
        .await
        .app;
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
    let app = crate::support::Fixture::new("smoke-ws", "test", "main", &[], &[])
        .await
        .app;

    // PUT workspace
    let body = serde_json::json!({"name": "demo", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .method("PUT")
        .uri("/api/workspaces/test/demo")
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
                .uri("/api/workspaces/test/demo")
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
    let app = crate::support::Fixture::new("smoke-node", "test", "main", &["demo", "ws1"], &["t"])
        .await
        .app;

    // PUT node under ws "demo" using path-based repo API
    // Created through POST on the parent (PUT only updates an existing node).
    let (status, text) = crate::support::create_at(&app, DEMO, "n1", "node1", "/node1", "t").await;
    assert_eq!(status, StatusCode::CREATED, "{text}");

    // GET node by path
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{DEMO}/node1"))
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
                .uri(format!("{DEMO}/node1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn query_endpoints() {
    let app = crate::support::Fixture::new(
        "smoke-query",
        "test",
        "main",
        &["demo", "ws1"],
        &["alpha", "beta"],
    )
    .await
    .app;

    // seed nodes
    for (id, name, path, parent, t) in [
        ("a", "A", "/a", None, "alpha"),
        ("b", "B", "/a/b", Some("/a"), "beta"),
        ("c", "C", "/a/c", Some("/a"), "beta"),
    ] {
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, DEMO, id, name, path, t).await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }

    // query by type
    let req = Request::builder()
        .method("POST")
        .uri(format!("{DEMO}/query"))
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

    // query by parent — `parent` is the parent node's ID ("a"), as the SDK sends it
    let req = Request::builder()
        .method("POST")
        .uri(format!("{DEMO}/query"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"parent":"a"})).unwrap(),
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
        .uri(format!("{DEMO}/query"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"parent":"a","nodeType":"beta"})).unwrap(),
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
        .uri(format!("{DEMO}/query"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"parent":"a","limit":1})).unwrap(),
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
        .uri(format!("{DEMO}/query"))
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
        .uri(format!("{DEMO}/query"))
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
    let app =
        crate::support::Fixture::new("smoke-upload", "test", "main", &["demo", "ws1"], &["t"])
            .await
            .app;

    // First create the node
    // (POST on the parent creates; `?deep=true` creates the missing /path/to)
    let node_body = serde_json::json!({
        "id": "test-node",
        "name": "node",
        "node_type": "t",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS1}/path/to?deep=true"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

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
        .uri(format!("{WS1}/path/to/node"))
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
                .uri(format!("{WS1}/path/to/node"))
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
    let app = crate::support::Fixture::new(
        "smoke-dsl",
        "test",
        "main",
        &["demo", "ws1"],
        &["alpha", "beta"],
    )
    .await
    .app;

    // seed nodes
    // The DSL endpoint evaluates the workspace's ROOT-level nodes (see
    // `post_query_dsl`), whose `parent` is "/".
    for (id, name, path, parent, t) in [
        ("a", "A", "/a", None::<&str>, "alpha"),
        ("b", "B", "/b", None, "beta"),
        ("c", "C", "/c", None, "beta"),
    ] {
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, DEMO, id, name, path, t).await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }

    // DSL: and [ { nodeType in ["beta"] }, { parent eq "/" } ] with limit 1
    let dsl = serde_json::json!({
        "and": [
            { "nodeType": { "in": ["beta"] } },
            { "parent": { "eq": "/" } }
        ],
        "order_by": { "path": "asc" },
        "limit": 1,
        "offset": 0
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{DEMO}/query/dsl"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&dsl).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Page<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.items.len(), 1);
}
