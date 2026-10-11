//! Reading inside a stored `.rap` by ranged reads (E24), over the real router:
//! `raisin:file`, `raisin:browse` and the legacy `/contents` listing, for a
//! streamed ZIP64-layout archive and a `.rap` nested inside it.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::collections::HashMap;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_binary::zip_stream::{Compression, ZipStreamWriter};
use raisin_binary::{BinaryStorage, FilesystemBinaryStorage};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};

use crate::support::{Fixture, TENANT};

const REPO: &str = "browserepo";
const BRANCH: &str = "main";
const MANIFEST: &[u8] = b"name: big-export\nversion: 1.0.0\n";

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (
        status,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

async fn archive() -> Vec<u8> {
    let mut inner = ZipStreamWriter::new(Vec::new());
    inner
        .write_entry("manifest.yaml", Compression::Deflate(6), b"name: inner\n")
        .await
        .unwrap();
    let inner = inner.finish().await.unwrap();

    let mut zip = ZipStreamWriter::new(Vec::new());
    zip.force_zip64(true);
    for i in 0..200 {
        zip.write_entry(
            &format!("content/stories/site/page-{i}.yaml"),
            Compression::Deflate(6),
            format!("title: Page {i}\n").repeat(50).as_bytes(),
        )
        .await
        .unwrap();
    }
    zip.write_entry("content/assets/nested.rap", Compression::Stored, &inner)
        .await
        .unwrap();
    zip.write_entry("manifest.yaml", Compression::Deflate(6), MANIFEST)
        .await
        .unwrap();
    zip.finish().await.unwrap()
}

#[tokio::test]
async fn files_and_listings_come_from_ranged_reads() {
    let f = Fixture::new(
        "pkg-browse",
        REPO,
        BRANCH,
        &["packages"],
        &["raisin:Package"],
    )
    .await;
    // The router's blob store (see `raisin_transport_http::router`).
    let bin = FilesystemBinaryStorage::new("./.data/uploads", Some("/files".into()));
    let stored = bin
        .put_bytes(
            &archive().await,
            Some("application/zip"),
            Some("rap"),
            None,
            Some(TENANT),
        )
        .await
        .unwrap();

    let mut resource = HashMap::new();
    resource.insert("key".to_string(), PropertyValue::String(stored.key.clone()));
    let mut props = HashMap::new();
    props.insert("resource".to_string(), PropertyValue::Object(resource));
    f.storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, "packages"),
            raisin_models::nodes::Node {
                id: "package-big-export".into(),
                name: "big-export".into(),
                path: "/big-export".into(),
                parent: Some("/".into()),
                node_type: "raisin:Package".into(),
                properties: props,
                ..Default::default()
            },
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let base = format!("/api/packages/{REPO}/{BRANCH}/head/big-export");
    let (status, body) = get(&f.app, &format!("{base}/raisin:file/manifest.yaml")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, MANIFEST);

    let (status, body) = get(
        &f.app,
        &format!("{base}/raisin:file/content/stories/site/page-7.yaml"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "title: Page 7\n".repeat(50).as_bytes());

    // Inside the nested archive.
    let (status, body) = get(
        &f.app,
        &format!("{base}/raisin:file/content/assets/nested.rap/manifest.yaml"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, b"name: inner\n");

    let (status, _) = get(&f.app, &format!("{base}/raisin:file/nope.yaml")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = get(
        &f.app,
        &format!("{base}/raisin:browse/content/stories/site"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listing: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(listing.as_array().unwrap().len(), 200);

    let (status, body) = get(&f.app, &format!("{base}/raisin:browse")).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<String> = serde_json::from_slice::<serde_json::Value>(&body)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["content", "manifest.yaml"]);

    let (status, body) = get(
        &f.app,
        &format!("/api/repos/{REPO}/packages/big-export/contents"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let contents: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(contents["total"], 202);

    let _ = bin.delete(&stored.key).await;
}

#[tokio::test]
async fn zip_bombs_stop_at_the_cap() {
    let f = Fixture::new("pkg-bomb", REPO, BRANCH, &["packages"], &["raisin:Package"]).await;
    // 80 MiB of zeros deflates to ~80 KB.
    let zeros = vec![0u8; 80 * 1024 * 1024];
    let mut inner = ZipStreamWriter::new(Vec::new());
    inner
        .write_entry("bomb.yaml", Compression::Deflate(9), &zeros)
        .await
        .unwrap();
    let inner = inner.finish().await.unwrap();
    let mut zip = ZipStreamWriter::new(Vec::new());
    zip.write_entry("bomb.yaml", Compression::Deflate(9), &zeros)
        .await
        .unwrap();
    zip.write_entry("nested.rap", Compression::Stored, &inner)
        .await
        .unwrap();
    zip.write_entry("manifest.yaml", Compression::Deflate(6), MANIFEST)
        .await
        .unwrap();
    let archive = zip.finish().await.unwrap();
    drop(zeros);
    assert!(archive.len() < 1_000_000);

    let bin = FilesystemBinaryStorage::new("./.data/uploads", Some("/files".into()));
    let stored = bin
        .put_bytes(
            &archive,
            Some("application/zip"),
            Some("rap"),
            None,
            Some(TENANT),
        )
        .await
        .unwrap();
    let mut resource = HashMap::new();
    resource.insert("key".to_string(), PropertyValue::String(stored.key.clone()));
    let mut props = HashMap::new();
    props.insert("resource".to_string(), PropertyValue::Object(resource));
    f.storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, "packages"),
            raisin_models::nodes::Node {
                id: "package-bomb".into(),
                name: "bomb".into(),
                path: "/bomb".into(),
                parent: Some("/".into()),
                node_type: "raisin:Package".into(),
                properties: props,
                ..Default::default()
            },
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let base = format!("/api/packages/{REPO}/{BRANCH}/head/bomb");

    // A top-level bomb: the response stops early (ratio guard), never 80 MiB.
    let res = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{base}/raisin:file/bomb.yaml"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    match res.into_body().collect().await {
        Ok(b) => assert!(b.to_bytes().len() < 80 * 1024 * 1024),
        Err(_) => {} // the stream was cut: the cap held
    }

    // A bomb inside a nested archive (read in memory) is refused.
    let (status, body) = get(&f.app, &format!("{base}/raisin:file/nested.rap/bomb.yaml")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );

    // The manifest beside them is still readable.
    let (status, body) = get(&f.app, &format!("{base}/raisin:file/manifest.yaml")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, MANIFEST);
    let _ = bin.delete(&stored.key).await;
}
