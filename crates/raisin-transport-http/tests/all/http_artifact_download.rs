//! A generated artifact's signed download (E22, E23), over the real router.
//!
//! - minting needs read access under row-level security, and the URL is
//!   bound to the blob's sha256 with the long artifact lifetime;
//! - the body streams with an exact `Content-Length`, `Accept-Ranges`, a
//!   strong `ETag` and `Repr-Digest`; a range is a `206` of exactly those
//!   bytes, an unsatisfiable one a `416`;
//! - `If-Range` with another blob's tag gets the whole file, never a splice;
//! - an unbound signature, or one bound to other content, is refused;
//! - an ordinary upload keeps its 300 s URL.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use sha2::Digest;
use tower::ServiceExt;

use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope, UpdateNodeOptions};

use crate::support::{Fixture, SIGNING_SECRET, TENANT};

const REPO: &str = "artrepo";
const BRANCH: &str = "main";
const WS: &str = "settings";
const BYTES: &[u8] = b"PK\x03\x04 a streamed archive, thirty-nine bytes!";

struct Res {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn send(app: &axum::Router, req: Request<Body>) -> Res {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Res {
        status,
        headers,
        body,
    }
}

fn sha_hex(b: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(b))
}

/// `/runs/<name>` with BYTES uploaded as `file`; with `artifact`, its envelope
/// records the sha256 the way a package build writes it.
async fn uploaded(f: &Fixture, name: &str, artifact: bool) {
    let base = format!("/api/repository/{REPO}/{BRANCH}/head/{WS}");
    let body = serde_json::json!({ "name": name, "node_type": "asset", "properties": {} });
    let res = send(
        &f.app,
        Request::builder()
            .method("POST")
            .uri(format!("{base}/runs?deep=true"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await;
    assert!(res.status.is_success(), "create: {}", res.status);

    let boundary = "XB";
    let mut buf: Vec<u8> = Vec::new();
    use std::io::Write;
    write!(buf, "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/zip\r\n\r\n").unwrap();
    buf.extend_from_slice(BYTES);
    write!(buf, "\r\n--{boundary}--\r\n").unwrap();
    let res = send(
        &f.app,
        Request::builder()
            .method("POST")
            .uri(format!("{base}/runs/{name}"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(buf))
            .unwrap(),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&res.body)
    );

    if artifact {
        let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
        let mut node = f
            .storage
            .nodes()
            .get_by_path(scope, &format!("/runs/{name}"), None)
            .await
            .unwrap()
            .unwrap();
        let Some(PropertyValue::Resource(r)) = node.properties.get_mut("file") else {
            panic!("no file resource");
        };
        // As a build writes it: the blob under this tenant's and repo's
        // artifact namespace, the digest on the envelope.
        let stored = raisin_binary::BinaryStorage::put_bytes(
            &raisin_binary::FilesystemBinaryStorage::new("./.data/uploads", None),
            BYTES,
            Some("application/zip"),
            Some("rap"),
            None,
            Some(&raisin_core::artifact_key_namespace(TENANT, REPO)),
        )
        .await
        .unwrap();
        let meta = r.metadata.get_or_insert_with(Default::default);
        meta.insert("sha256".into(), PropertyValue::String(sha_hex(BYTES)));
        meta.insert("storage_key".into(), PropertyValue::String(stored.key));
        f.storage
            .nodes()
            .update(
                scope,
                node,
                UpdateNodeOptions {
                    validate_schema: false,
                    allow_type_change: false,
                    operation_meta: None,
                },
            )
            .await
            .unwrap();
    }
}

/// Mint through `raisin:sign` as `app`'s caller; returns (status, url, expires_at).
async fn sign(app: &axum::Router, name: &str) -> (StatusCode, String, String) {
    let res = send(
        app,
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/repository/{REPO}/{BRANCH}/head/{WS}/runs/{name}/raisin:sign"
            ))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"command":"download"}"#))
            .unwrap(),
    )
    .await;
    let v: serde_json::Value = serde_json::from_slice(&res.body).unwrap_or_default();
    (
        res.status,
        v["url"].as_str().unwrap_or_default().to_string(),
        v["expires_at"].as_str().unwrap_or_default().to_string(),
    )
}

async fn get(app: &axum::Router, url: &str, headers: &[(&str, &str)]) -> Res {
    let mut req = Request::builder().uri(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    send(app, req.body(Body::empty()).unwrap()).await
}

fn secs_until(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .signed_duration_since(chrono::Utc::now())
        .num_seconds()
}

#[tokio::test]
async fn an_artifact_downloads_streamed_ranged_and_resumable() {
    let f = Fixture::new(
        "artifact-dl",
        REPO,
        BRANCH,
        &[WS],
        &["asset", "raisin:Folder"],
    )
    .await;
    uploaded(&f, "pkg.rap", true).await;

    let (status, url, expires_at) = sign(&f.app, "pkg.rap").await;
    assert_eq!(status, StatusCode::OK);
    // The artifact lifetime (12 h), not the 300 s of an ordinary asset.
    let ttl = secs_until(&expires_at);
    assert!((12 * 3600 - 60..=12 * 3600).contains(&ttl), "ttl {ttl}");

    let etag = format!("\"sha256:{}\"", sha_hex(BYTES));
    let full = get(&f.raw, &url, &[]).await;
    assert_eq!(
        full.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&full.body)
    );
    assert_eq!(full.body, BYTES);
    let h = |r: &Res, k: &str| r.headers.get(k).map(|v| v.to_str().unwrap().to_string());
    assert_eq!(h(&full, "content-length"), Some(BYTES.len().to_string()));
    assert_eq!(h(&full, "accept-ranges").as_deref(), Some("bytes"));
    assert_eq!(h(&full, "etag"), Some(etag.clone()));
    let digest = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(BYTES))
    };
    assert_eq!(h(&full, "repr-digest"), Some(format!("sha-256=:{digest}:")));
    assert!(h(&full, "content-disposition")
        .unwrap()
        .starts_with("attachment"));
    let cc = h(&full, "cache-control").unwrap();
    assert!(
        cc.starts_with("private, max-age=4") && cc.ends_with("no-transform"),
        "{cc}"
    );

    // A range: exactly those bytes.
    let part = get(&f.raw, &url, &[("range", "bytes=10-19")]).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body, &BYTES[10..20]);
    assert_eq!(
        h(&part, "content-range"),
        Some(format!("bytes 10-19/{}", BYTES.len()))
    );
    assert_eq!(h(&part, "content-length").as_deref(), Some("10"));

    // A resume (`curl -C -`): an open-ended range with the validator it had.
    let resume = get(&f.raw, &url, &[("range", "bytes=20-"), ("if-range", &etag)]).await;
    assert_eq!(resume.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(resume.body, &BYTES[20..]);

    // The file changed under the client: the whole, current file.
    let other = format!("\"sha256:{}\"", "0".repeat(64));
    let stale = get(
        &f.raw,
        &url,
        &[("range", "bytes=20-"), ("if-range", &other)],
    )
    .await;
    assert_eq!(stale.status, StatusCode::OK);
    assert_eq!(stale.body, BYTES);

    // Past the end.
    let beyond = get(&f.raw, &url, &[("range", "bytes=1000-2000")]).await;
    assert_eq!(beyond.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        h(&beyond, "content-range"),
        Some(format!("bytes */{}", BYTES.len()))
    );
}

#[tokio::test]
async fn an_artifact_url_is_bound_to_its_blob() {
    let f = Fixture::new(
        "artifact-bound",
        REPO,
        BRANCH,
        &[WS],
        &["asset", "raisin:Folder"],
    )
    .await;
    uploaded(&f, "pkg.rap", true).await;
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let mint = |sha: Option<&str>| {
        raisin_core::build_signed_asset_url_bound(
            SIGNING_SECRET.as_bytes(),
            TENANT,
            REPO,
            BRANCH,
            WS,
            "/runs/pkg.rap",
            "file",
            "download",
            exp,
            None,
            sha,
        )
        .url
    };
    // Bound to this blob: works.
    assert_eq!(
        get(&f.raw, &mint(Some(&sha_hex(BYTES))), &[]).await.status,
        StatusCode::OK
    );
    // Unbound (an old-style URL) or bound to another blob: refused.
    assert_eq!(
        get(&f.raw, &mint(None), &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&f.raw, &mint(Some(&"f".repeat(64))), &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn signing_needs_read_access_and_ordinary_assets_keep_300s() {
    let f = Fixture::new(
        "artifact-sign",
        REPO,
        BRANCH,
        &[WS],
        &["asset", "raisin:Folder"],
    )
    .await;
    uploaded(&f, "pkg.rap", true).await;
    uploaded(&f, "plain.bin", false).await;

    // Anonymous may read nothing here, so it may mint nothing.
    let (status, url, _) = sign(&f.raw, "pkg.rap").await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND,
        "{status}"
    );
    assert!(url.is_empty());

    let (status, url, expires_at) = sign(&f.app, "plain.bin").await;
    assert_eq!(status, StatusCode::OK);
    let ttl = secs_until(&expires_at);
    assert!((240..=300).contains(&ttl), "ttl {ttl}");
    let res = get(&f.raw, &url, &[]).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.headers.get("etag").is_none(), "no digest, no validator");
}

#[tokio::test]
async fn a_resource_pointing_at_another_tenants_blob_is_neither_served_nor_signed() {
    use raisin_binary::BinaryStorage;
    let f = Fixture::new(
        "artifact-foreign",
        REPO,
        BRANCH,
        &[WS],
        &["asset", "raisin:Folder"],
    )
    .await;
    uploaded(&f, "victim.bin", false).await;
    // Another tenant's blob in the same store…
    let bin = raisin_binary::FilesystemBinaryStorage::new("./.data/uploads", None);
    let foreign = bin
        .put_bytes(
            b"another tenant's file",
            None,
            Some("bin"),
            None,
            Some("other-tenant"),
        )
        .await
        .unwrap();
    // …and a node of THIS tenant whose Resource is rewritten to name it.
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let mut node = f
        .storage
        .nodes()
        .get_by_path(scope, "/runs/victim.bin", None)
        .await
        .unwrap()
        .unwrap();
    let Some(PropertyValue::Resource(r)) = node.properties.get_mut("file") else {
        panic!("no file resource");
    };
    r.metadata.get_or_insert_with(Default::default).insert(
        "storage_key".into(),
        PropertyValue::String(foreign.key.clone()),
    );
    f.storage
        .nodes()
        .update(
            scope,
            node,
            UpdateNodeOptions {
                validate_schema: false,
                allow_type_change: false,
                operation_meta: None,
            },
        )
        .await
        .unwrap();

    let read = get(
        &f.app,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{WS}/runs/victim.bin/raisin:display"),
        &[],
    )
    .await;
    assert_eq!(read.status, StatusCode::NOT_FOUND);
    assert_ne!(read.body, b"another tenant's file");
    let (status, url, _) = sign(&f.app, "victim.bin").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(url.is_empty());
    let _ = bin.delete(&foreign.key).await;
}
