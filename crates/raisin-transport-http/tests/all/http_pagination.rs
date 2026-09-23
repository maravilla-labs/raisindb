#![cfg(not(feature = "s3"))]
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The HEAD route of workspace `ws` in repository `test`.
const WS: &str = "/api/repository/test/main/head/ws";

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

/// Repository `test` with node type `t` and the given workspaces, sent as the
/// operator — see `support`.
async fn app(workspaces: &[&str]) -> axum::Router {
    crate::support::Fixture::new("pagination", "test", "main", workspaces, &["t"])
        .await
        .app
}

#[tokio::test]
async fn pagination_under_query_parent_limit_1() {
    let app = app(&["ws"]).await;
    // seed /p with 3 children in stable order
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("a", "a", "/p/a", Some("/p")),
        ("b", "b", "/p/b", Some("/p")),
        ("c", "c", "/p/c", Some("/p")),
    ] {
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, WS, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }

    // POST /:ws/query with parent and limit=1, then paginate
    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        // `parent` is the parent node's ID (here "p"), as the client SDK sends it.
        let req_body = serde_json::json!({"parent":"p","limit":1,"offset":offset});
        let req = Request::builder()
            .method("POST")
            .uri(format!("{WS}/query"))
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
    let app = app(&[]).await;
    // create 3 workspaces out of order
    for name in ["c", "a", "b"] {
        let body = serde_json::json!({"name": name, "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/workspaces/test/{}", name))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
    }

    // GET /workspaces?limit=2&offset=0
    let req = Request::builder()
        .uri("/api/workspaces/test?limit=2&offset=0")
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
        .uri("/api/workspaces/test?limit=2&offset=2")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let page: Page<raisin_models::workspace::Workspace> = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<_> = page.items.iter().map(|w| w.name.clone()).collect();
    assert_eq!(names, vec!["c"]);
    assert_eq!(page.page.next_offset, None);
}
