//! Administrative surfaces refuse callers without admin rights and keep
//! working for the operator (superadmin bearer, which `optional_auth_middleware`
//! turns into `AuthContext::system()` exactly like an admin JWT or an API key).

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::support::{admin, create_repository, fresh_storage, send};

const REPO: &str = "gates";

/// The test router, but with the RocksDB handle `/api/sql` needs (the plain
/// `router()` leaves it out, so every SQL request would fail before any gate).
async fn app(label: &str) -> axum::Router {
    let storage = fresh_storage(label);
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

fn json_body(v: &Value) -> Body {
    Body::from(serde_json::to_vec(v).unwrap())
}

async fn sql(app: &axum::Router, statement: &str, as_admin: bool) -> (StatusCode, String) {
    let body = json!({ "sql": statement });
    let mut req = post(&format!("/api/sql/{REPO}"));
    if as_admin {
        req = admin(req);
    }
    send(app, req.body(json_body(&body)).unwrap()).await
}

/// `POST /api/files/{repo}/run` executes caller-supplied code as the system.
#[tokio::test]
async fn run_file_needs_an_administrator() {
    let app = app("run-file").await;
    let body = json!({
        "code": "export default function main() { return 1 }",
        "file_name": "probe.js",
        "handler": "default",
    });
    let uri = format!("/api/files/{REPO}/run");

    // Anonymous.
    let (status, text) = send(&app, post(&uri).body(json_body(&body)).unwrap()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    // A bearer that validates as nothing is anonymous too.
    let req = post(&uri)
        .header("authorization", "Bearer not-a-real-token")
        .body(json_body(&body))
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::FORBIDDEN);
    // An API-key-shaped bearer that does not validate.
    let req = post(&uri)
        .header("authorization", "Bearer raisin_forged")
        .body(json_body(&body))
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::FORBIDDEN);

    // The operator (admin console, CLI) still runs files.
    let (status, text) = send(&app, admin(post(&uri)).body(json_body(&body)).unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// AI/embedding config and branch changes over `/api/sql`.
#[tokio::test]
async fn admin_sql_statements_need_an_administrator() {
    let app = app("admin-sql").await;
    for statement in [
        "ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE = '0.9'",
        "ALTER EMBEDDING CONFIG SET BASE_URL = 'https://attacker.example'",
        "REGENERATE EMBEDDINGS",
        "DROP BRANCH 'main'",
        "CREATE BRANCH 'gate-probe' FROM 'main'",
    ] {
        let (status, text) = sql(&app, statement, false).await;
        // `/api/sql` reports every engine error as 400; the refusal is the
        // engine's Forbidden, not some other failure.
        assert!(
            !status.is_success() && text.contains("Forbidden"),
            "anonymous {statement} must be refused: {status} {text}"
        );
    }

    // The operator path still works: the statement embedder-switch.sh sends.
    let (status, text) = sql(
        &app,
        "ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE = '0.78'",
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let (status, text) = sql(&app, "SHOW EMBEDDING CONFIG", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("0.78"), "{text}");
    let (status, text) = sql(&app, "CREATE BRANCH 'gate-probe' FROM 'main'", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let (status, text) = sql(&app, "DROP BRANCH IF EXISTS 'gate-probe'", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    // ...and `main` survived the anonymous DROP.
    let (status, text) = sql(&app, "SHOW BRANCHES", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("main"), "{text}");
}
