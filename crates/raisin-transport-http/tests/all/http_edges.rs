#![cfg(not(feature = "s3"))]
//! Edge cases of move / copy / rename / deep listing over the current
//! repository API. The fixture creates repository `test` with workspace `ws`
//! and sends as the operator — see `support`.

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The HEAD route of workspace `ws` in repository `test`.
const WS: &str = "/api/repository/test/main/head/ws";

async fn app() -> axum::Router {
    crate::support::Fixture::new("edges", "test", "main", &["ws"], &["t"])
        .await
        .app
}

#[tokio::test]
async fn move_into_own_descendant_is_rejected() {
    let app = app().await;
    // seed /a and /a/b
    for (id, name, path, parent) in [("a", "a", "/a", None), ("b", "b", "/a/b", Some("/a"))] {
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, WS, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }
    // move /a -> /a/b/a should fail
    let body = serde_json::json!({"targetPath":"/a/b/a"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/a?command=move"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // Refused as the caller's mistake: 400, not a server error.
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
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
        // Created through POST on the parent (PUT only updates an existing node).
        let _ = parent;
        let (status, text) = crate::support::create_at(&app, WS, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }
    // copy_tree /a -> /a/b/a should fail
    let body = serde_json::json!({"targetPath":"/a/b/a"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/a?command=copy_tree"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // Refused as the caller's mistake: 400, not a server error.
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rename_root_and_invalid_name() {
    let app = app().await;
    // seed root child /x
    let (status, text) = crate::support::create_at(&app, WS, "x", "x", "/x", "t").await;
    assert!(status.is_success(), "seeding /x: {status} {text}");

    // rename to valid new name -> /y
    let body = serde_json::json!({"newName":"y"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/x?command=rename"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    // verify
    let req = Request::builder()
        .uri(format!("{WS}/y"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // invalid name containing slash
    let body = serde_json::json!({"newName":"bad/name"});
    let req = Request::builder()
        .method("POST")
        .uri(format!("{WS}/y?command=rename"))
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
        .uri(format!("{WS}/?level=3&flatten=true"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let flat: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(flat, serde_json::json!([]));

    // create a chain 15 deep under /p (a parent must exist before its children)
    let (status, text) = crate::support::create_at(&app, WS, "p", "p", "/p", "t").await;
    assert!(status.is_success(), "seeding /p: {status} {text}");
    for i in 0..15u32 {
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
        let (status, text) =
            crate::support::create_at(&app, WS, &format!("id{}", i), &name, &path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
    }
    // A requested level is capped at 10. Levels count from 0 (the direct
    // children), so level 10 reaches 11 generations: asking for 20 below n0
    // yields n1..n11 of the 14 that exist.
    // `format=map` selects the keyed forms, of which `flatten` is the flat list
    // (the default `array` form nests and ignores `flatten`).
    let req = Request::builder()
        .uri(format!("{WS}/p/n0?level=20&flatten=true&format=map"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let flat: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    // flat is a list of the descendants -> should be 11 (n1..n11)
    assert_eq!(flat.as_array().map(|a| a.len()).unwrap_or(0), 11, "{flat}");
}
