#![cfg(all(not(feature = "s3"), feature = "storage-rocksdb"))]
//! `PATCH /api/repositories/{repo}/translation-config` with `default_language`.
//!
//! The contract: a change answers 200 with the stored config and the queued
//! full-text rebuilds under `reindex_jobs`; overlays in the new default answer
//! 409 naming the language and the overlay count and change nothing; the same
//! default is a no-op 200.

use std::collections::HashMap;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_storage::{Storage, TranslationRepository};
use serde_json::{json, Value};

use crate::support::{send, Fixture, TENANT};

const REPO: &str = "langrepo";
const BRANCH: &str = "main";

async fn fixture(label: &str) -> Fixture {
    Fixture::new(label, REPO, BRANCH, &[], &[]).await
}

async fn patch(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/repositories/{REPO}/translation-config"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, text) = send(app, req).await;
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, json)
}

async fn get_config(app: &axum::Router) -> Value {
    let req = Request::builder()
        .uri(format!("/api/repositories/{REPO}/translation-config"))
        .body(Body::empty())
        .unwrap();
    let (status, text) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn changing_the_default_returns_the_config_and_the_queued_rebuilds() {
    let fx = fixture("deflang-change").await;

    let (status, body) = patch(&fx.app, json!({ "default_language": "DE" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["default_language"], "de", "normalized: {body}");
    assert_eq!(body["previous_default_language"], "en");
    assert_eq!(
        body["supported_languages"],
        json!(["en", "de"]),
        "the new default is added and the old one kept"
    );
    assert_eq!(body["default_branch"], BRANCH);
    let jobs = body["reindex_jobs"].as_array().expect("reindex_jobs");
    assert_eq!(jobs.len(), 1, "{body}");
    assert_eq!(jobs[0]["branch"], BRANCH);
    assert!(!jobs[0]["job_id"].as_str().unwrap().is_empty());

    let config = get_config(&fx.app).await;
    assert_eq!(config["default_language"], "de");
    assert_eq!(config["supported_languages"], json!(["en", "de"]));
}

#[tokio::test]
async fn the_new_default_is_added_to_an_explicit_supported_list() {
    let fx = fixture("deflang-explicit").await;

    let (status, body) = patch(
        &fx.app,
        json!({ "default_language": "fr", "supported_languages": ["fr", "it"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["default_language"], "fr");
    assert_eq!(body["supported_languages"], json!(["fr", "it"]));
}

#[tokio::test]
async fn the_same_default_is_a_no_op() {
    let fx = fixture("deflang-noop").await;

    let (status, body) = patch(&fx.app, json!({ "default_language": "en" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["default_language"], "en");
    assert_eq!(body["supported_languages"], json!(["en"]));
    assert_eq!(body["reindex_jobs"], json!([]));
    assert!(body.get("previous_default_language").is_none(), "{body}");
}

#[tokio::test]
async fn overlays_in_the_new_default_refuse_the_change_with_409() {
    let fx = fixture("deflang-conflict").await;

    let locale = LocaleCode::parse("de").unwrap();
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Titel".to_string()),
    );
    fx.storage
        .translations()
        .store_translation(
            TENANT,
            REPO,
            BRANCH,
            "content",
            "node-1",
            &locale,
            &LocaleOverlay::properties(data),
            &TranslationMeta::system(locale.clone(), HLC::new(100, 0), "seed".to_string()),
        )
        .await
        .unwrap();

    let (status, body) = patch(&fx.app, json!({ "default_language": "de" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "DEFAULT_LANGUAGE_CONFLICT");
    assert_eq!(body["language"], "de");
    assert_eq!(body["overlay_count"], 1);

    let config = get_config(&fx.app).await;
    assert_eq!(config["default_language"], "en", "nothing changed");
    assert_eq!(config["supported_languages"], json!(["en"]));
}

#[tokio::test]
async fn an_invalid_language_code_is_rejected() {
    let fx = fixture("deflang-invalid").await;

    let (status, body) = patch(&fx.app, json!({ "default_language": "german" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(get_config(&fx.app).await["default_language"], "en");
}

#[tokio::test]
async fn updating_only_supported_languages_answers_with_the_config() {
    let fx = fixture("deflang-supported").await;

    let (status, body) = patch(&fx.app, json!({ "supported_languages": ["fr"] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["default_language"], "en");
    assert_eq!(
        body["supported_languages"],
        json!(["fr", "en"]),
        "the default is always kept"
    );
    assert_eq!(body["reindex_jobs"], json!([]));
}
