//! `POST /api/management/{repo}/repairs/{repair}` enqueues an index repair on
//! this node and forwards it to every peer; `GET .../status` reports each
//! node's persisted state. A peer that cannot be reached is reported, not
//! fatal.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use axum::{body::Body, http::Request, http::StatusCode};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::support::{admin, create_repository, init_env, send};

const REPO: &str = "repairs";
const NODE: &str = "node-a";
/// Nothing listens on port 1, so the connection is refused at once.
const DEAD_PEER: &str = "http://127.0.0.1:1";

/// A router over a node `node-a` whose only peer, `node-b`, is down.
async fn app(label: &str) -> axum::Router {
    init_env();
    let path = format!("/tmp/raisin-http-test-{label}-{}", nanoid::nanoid!(8));
    let mut config = raisin_rocksdb::RocksDBConfig::development()
        .with_path(&path)
        .with_peers(vec![raisin_rocksdb::ReplicationPeerConfig::new(
            "node-b", DEAD_PEER,
        )]);
    config.cluster_node_id = Some(NODE.to_string());
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::with_config(config).unwrap());
    let bin = Arc::new(raisin_binary::FilesystemBinaryStorage::new(
        "./.data/uploads",
        Some("/files".into()),
    ));
    let audit = Arc::new(storage.audit_repository());
    let adapter = Arc::new(raisin_core::RepoAuditAdapter::new(audit.clone()));
    let ws_svc =
        Arc::new(raisin_core::services::workspace_service::WorkspaceService::new(storage.clone()));
    let (app, _state) = raisin_transport_http::router_with_bin_and_audit(
        storage.clone(),
        ws_svc,
        bin,
        audit,
        adapter,
        false,
        false,
        "0.0.0-test".to_string(),
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        Some(storage.clone()),
        None,
        None,
        None,
        None,
    );
    create_repository(&app, REPO, "main").await;
    app
}

fn post(uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
}

fn node<'a>(resp: &'a Value, id: &str) -> &'a Value {
    resp["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["node_id"] == id)
        .unwrap_or_else(|| panic!("no node {id} in {resp}"))
}

#[tokio::test]
async fn repair_routes_need_an_administrator() {
    let app = app("repair-gate").await;
    let body = json!({ "branch": "main", "dry_run": true });
    let uri = format!("/api/management/{REPO}/repairs/ordered_children");

    let req = post(&uri).body(Body::from(body.to_string())).unwrap();
    let (status, text) = send(&app, req).await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "anonymous POST: {status} {text}"
    );

    // The status read must not fall through to the open schema reads.
    let req = Request::builder()
        .uri(format!("{uri}/status"))
        .body(Body::empty())
        .unwrap();
    let (status, text) = send(&app, req).await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "anonymous GET status: {status} {text}"
    );

    // An unknown repair is a 400, not a silently empty job.
    let req = admin(post(&format!("/api/management/{REPO}/repairs/nonsense")))
        .body(Body::from(body.to_string()))
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn enqueue_and_status_report_every_node_including_an_unreachable_peer() {
    let app = app("repair-fanout").await;
    let body = json!({ "branch": "main", "dry_run": true });
    let uri = format!("/api/management/{REPO}/repairs/ordered_children");

    let req = admin(post(&uri))
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let resp: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(resp["nodes"].as_array().unwrap().len(), 2, "{resp}");
    let local = node(&resp, NODE);
    assert_eq!(local["status"], "enqueued", "{resp}");
    assert!(local["job_id"].as_str().is_some_and(|j| !j.is_empty()));
    assert_eq!(node(&resp, "node-b")["status"], "unreachable", "{resp}");

    // A forwarded request acts on this node only.
    let req = admin(post(&format!("{uri}?local_only=true")))
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let resp: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(resp["nodes"].as_array().unwrap().len(), 1, "{resp}");

    // Status: this node reports its branches; the dead peer is named.
    let req = admin(Request::builder().uri(format!("{uri}/status")))
        .body(Body::empty())
        .unwrap();
    let (status, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let resp: Value = serde_json::from_str(&text).unwrap();
    let local = node(&resp, NODE);
    assert_eq!(local["status"], "reported", "{resp}");
    assert!(
        local["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["branch"] == "main"),
        "{resp}"
    );
    assert_eq!(node(&resp, "node-b")["status"], "unreachable", "{resp}");
}
