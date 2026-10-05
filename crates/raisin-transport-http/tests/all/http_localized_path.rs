#![cfg(all(not(feature = "s3"), feature = "storage-rocksdb"))]
//! `GET /api/repository/{repo}/{branch}/head/{ws}/by-localized-path/{locale}/{*path}`
//! (plan Phase 12): the translated node, `canonical_path`,
//! `canonical_localized_path`, hreflang `alternates` and the 301 hint; 404
//! for a path that does not resolve.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{body::Body, http::Request, http::StatusCode};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::{RepositoryManagementRepository, Storage};
use serde_json::Value;

use crate::support::{create_at, send, Fixture, TENANT};

const REPO: &str = "lpath";
const BRANCH: &str = "main";
const WS: &str = "pages";

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let (status, text) = send(app, req).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_node_by_its_localized_path() {
    let fx = Fixture::new("lpath", REPO, BRANCH, &[WS], &["t"]).await;
    let mut config = fx
        .storage
        .repository_management()
        .get_repository(TENANT, REPO)
        .await
        .unwrap()
        .unwrap()
        .config;
    config.supported_languages = vec!["en".into(), "fr".into()];
    fx.storage
        .repository_management()
        .update_repository_config(TENANT, REPO, config)
        .await
        .unwrap();
    let base = format!("/api/repository/{REPO}/{BRANCH}/head/{WS}");
    for (id, name, path, translated) in [
        ("p1", "products", "/products", "produits"),
        ("c1", "chair", "/products/chair", "chaise"),
    ] {
        let (status, text) = create_at(&fx.app, &base, id, name, path, "t").await;
        assert!(status.is_success(), "seeding {path}: {status} {text}");
        let data = HashMap::from([(
            JsonPointer::new("/__node_name"),
            PropertyValue::String(translated.to_string()),
        )]);
        raisin_core::TranslationService::new(Arc::clone(&fx.storage))
            .update_translation(
                TENANT,
                REPO,
                BRANCH,
                WS,
                id,
                &LocaleCode::parse("fr").unwrap(),
                data,
                "test",
                None,
            )
            .await
            .unwrap();
    }

    let (status, body) = get(
        &fx.app,
        &format!("{base}/by-localized-path/fr/produits/chaise"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["node"]["id"], "c1");
    assert_eq!(body["canonical_path"], "/products/chair");
    assert_eq!(body["canonical_localized_path"], "/produits/chaise");
    assert_eq!(body["redirect"], false);
    assert_eq!(body["alternates"]["en"], "/products/chair");
    assert_eq!(body["alternates"]["fr"], "/produits/chaise");

    // A canonical name where the node has its own translated name: resolved,
    // 301 hint.
    let (status, body) = get(
        &fx.app,
        &format!("{base}/by-localized-path/fr/products/chair"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["redirect"], true);
    assert_eq!(body["canonical_localized_path"], "/produits/chaise");

    let (status, _) = get(
        &fx.app,
        &format!("{base}/by-localized-path/fr/produits/nope"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The node route still answers the canonical path.
    let (status, body) = get(&fx.app, &format!("{base}/products/chair")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// `PUT /api/repositories/{repo}` sets `localized_names` and keeps it when a
/// later update does not mention it.
#[tokio::test]
async fn localized_names_config_is_set_and_kept_by_repository_updates() {
    let fx = Fixture::new("lpath-cfg", REPO, BRANCH, &[WS], &["t"]).await;
    let put = |body: Value| {
        Request::builder()
            .method("PUT")
            .uri(format!("/api/repositories/{REPO}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    };
    let body = serde_json::json!({
        "supported_languages": ["en", "fr"],
        "localized_names": { "enforce_unique": true }
    });
    let (status, text) = send(&fx.app, put(body)).await;
    assert!(status.is_success(), "{status} {text}");
    let (status, text) = send(&fx.app, put(serde_json::json!({ "description": "x" }))).await;
    assert!(status.is_success(), "{status} {text}");
    let config = fx
        .storage
        .repository_management()
        .get_repository(TENANT, REPO)
        .await
        .unwrap()
        .unwrap()
        .config;
    assert!(config.localized_names.enforce_unique);
    assert_eq!(config.description.as_deref(), Some("x"));
}
