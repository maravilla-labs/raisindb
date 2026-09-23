#![cfg(not(feature = "s3"))]
//! Name collisions on rename / move over the current repository API. The
//! fixture creates repository `test` with workspace `ws` and sends as the
//! operator — see `support`.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

/// The HEAD route of workspace `ws` in repository `test`.
const WS: &str = "/api/repository/test/main/head/ws";

async fn app() -> axum::Router {
    crate::support::Fixture::new("collisions", "test", "main", &["ws"], &["t"])
        .await
        .app
}

#[tokio::test]
async fn rename_conflict_is_rejected() {
    let app = app().await;
    // seed /p with a and b
    for (id, name, path, parent) in [
        ("p", "p", "/p", None),
        ("a", "a", "/p/a", Some("/p")),
        ("b", "b", "/p/b", Some("/p")),
    ] {
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, WS, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }

    // repo command rename: /p/a -> new_name "b" (conflict)
    let body = serde_json::json!({"newName":"b"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/p/a?command=rename"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // A name collision is the caller's mistake: 400, not a server error.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn move_conflict_is_rejected() {
    let app = app().await;
    // seed /q with a and b
    for (id, name, path, parent) in [
        ("q", "q", "/q", None),
        ("a", "a", "/q/a", Some("/q")),
        ("b", "b", "/q/b", Some("/q")),
    ] {
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, WS, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }

    // repo command move: move /q/a to /q/b (conflict)
    let body = serde_json::json!({"targetPath":"/q/b"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/q/a?command=move"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // A name collision is the caller's mistake: 400, not a server error.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
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
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, WS, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }

    // move /src/a -> /dst/a (same name conflict)
    let body = serde_json::json!({"targetPath":"/dst/a"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/src/a?command=move"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // A name collision is the caller's mistake: 400, not a server error.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
