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
    app_with_credentials(label).await.0
}

/// An admin JWT and an API key minted by the router's own auth service.
struct Credentials {
    admin_jwt: String,
    api_key: String,
}

async fn app_with_credentials(label: &str) -> (axum::Router, Credentials) {
    let storage = fresh_storage(label);
    let auth = Arc::new(raisin_rocksdb::AuthService::new(
        raisin_rocksdb::AdminUserStore::new(storage.db().clone()),
        "http-admin-gates-jwt-secret".to_string(),
    ));
    let user = auth
        .create_user(
            crate::support::TENANT.to_string(),
            "gate-admin".to_string(),
            None,
            "Gate-Admin-Passw0rd!".to_string(),
            raisin_models::admin_user::AdminAccessFlags {
                console_login: true,
                cli_access: true,
                api_access: true,
                pgwire_access: true,
                can_impersonate: false,
            },
        )
        .unwrap();
    let credentials = Credentials {
        admin_jwt: auth.generate_token(&user).unwrap(),
        api_key: auth
            .create_api_key(crate::support::TENANT, &user.user_id, "gates")
            .unwrap()
            .1,
    };
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
        Some(auth),
        None,
        None,
        None,
    );
    create_repository(&app, REPO, "main").await;
    (app, credentials)
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

fn bearer(req: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    req.header("authorization", format!("Bearer {token}"))
}

/// The management, replication and AI-config REST routes had no gate.
#[tokio::test]
async fn rest_admin_routes_refuse_anonymous_callers_and_admit_admin_credentials() {
    let (app, creds) = app_with_credentials("rest-admin").await;
    let t = crate::support::TENANT;
    let embeddings = json!({
        "enabled": false, "provider": "OpenAI", "model": "text-embedding-3-small",
        "dimensions": 8, "include_name": true, "include_path": true,
    });

    // Anonymous: every administrative route is refused, nothing changes.
    let anonymous: Vec<(&str, String, Option<Value>)> = vec![
        (
            "POST",
            format!("/api/tenants/{t}/embeddings/config"),
            Some(embeddings.clone()),
        ),
        ("GET", format!("/api/tenants/{t}/embeddings/config"), None),
        (
            "PUT",
            format!("/api/tenants/{t}/ai/config"),
            Some(json!({"providers": []})),
        ),
        ("GET", format!("/api/tenants/{t}/ai/config"), None),
        (
            "DELETE",
            format!("/api/management/repositories/{t}/{REPO}/branches/main"),
            None,
        ),
        (
            "POST",
            format!("/api/management/repositories/{t}/{REPO}/branches"),
            Some(json!({"name": "x"})),
        ),
        (
            "GET",
            format!("/api/replication/{t}/{REPO}/operations"),
            None,
        ),
        (
            "POST",
            format!("/api/replication/{t}/{REPO}/operations/batch"),
            Some(json!({"operations": []})),
        ),
        (
            "GET",
            format!("/api/replication/{t}/{REPO}/vector-clock"),
            None,
        ),
        ("DELETE", format!("/api/repositories/{REPO}"), None),
        (
            "POST",
            "/api/repositories".to_string(),
            Some(json!({"repo_id": "sneaky"})),
        ),
        ("GET", "/api/management/registry/tenants".to_string(), None),
        (
            "POST",
            "/api/management/system-definitions/reload".to_string(),
            None,
        ),
        (
            "POST",
            format!("/api/management/{REPO}/main/nodetypes"),
            Some(json!({})),
        ),
    ];
    for (method, uri, body) in &anonymous {
        let req = Request::builder()
            .method(*method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.as_ref().map(json_body).unwrap_or_else(Body::empty))
            .unwrap();
        let (status, text) = send(&app, req).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
            "anonymous {method} {uri}: {status} {text}"
        );
    }

    // main survived, and the admin credentials still work: the admin JWT (the
    // console, the CLI), an API key (scripts/embedder-switch.sh), and the
    // superadmin bearer (Flightdeck, maravilla dev, a replication peer).
    for token in [
        creds.admin_jwt.as_str(),
        creds.api_key.as_str(),
        crate::support::ADMIN_TOKEN,
    ] {
        let req = bearer(
            Request::builder().uri(format!(
                "/api/management/repositories/{t}/{REPO}/branches/main"
            )),
            token,
        )
        .body(Body::empty())
        .unwrap();
        let (status, text) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{text}");

        let req = bearer(
            Request::builder()
                .method("POST")
                .uri(format!("/api/tenants/{t}/embeddings/config"))
                .header("content-type", "application/json"),
            token,
        )
        .body(json_body(&embeddings))
        .unwrap();
        let (status, text) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{text}");

        let req = bearer(
            Request::builder().uri(format!("/api/tenants/{t}/ai/config")),
            token,
        )
        .body(Body::empty())
        .unwrap();
        let (status, text) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{text}");

        let req = bearer(
            Request::builder()
                .method("POST")
                .uri(format!("/api/management/repositories/{t}/{REPO}/branches"))
                .header("content-type", "application/json"),
            token,
        )
        .body(json_body(&json!({"name": "gate-branch"})))
        .unwrap();
        let (status, text) = send(&app, req).await;
        assert!(status.is_success(), "create branch: {status} {text}");
        let req = bearer(
            Request::builder().method("DELETE").uri(format!(
                "/api/management/repositories/{t}/{REPO}/branches/gate-branch"
            )),
            token,
        )
        .body(Body::empty())
        .unwrap();
        let (status, text) = send(&app, req).await;
        assert!(status.is_success(), "delete branch: {status} {text}");
    }

    // A replication peer presents the superadmin bearer.
    let req = bearer(
        Request::builder().uri(format!("/api/replication/{t}/{REPO}/vector-clock")),
        crate::support::ADMIN_TOKEN,
    )
    .body(Body::empty())
    .unwrap();
    let (status, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // Public reads the site renderers make are unchanged.
    let req = Request::builder()
        .uri(format!("/api/repositories/{REPO}/translation-config"))
        .body(Body::empty())
        .unwrap();
    let (status, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// An admin credential of one tenant cannot act on another.
#[tokio::test]
async fn an_admin_credential_cannot_reach_another_tenant() {
    let (app, creds) = app_with_credentials("rest-cross-tenant").await;
    let req = bearer(
        Request::builder()
            .method("POST")
            .uri("/api/tenants/someone-else/embeddings/config")
            .header("content-type", "application/json"),
        &creds.api_key,
    )
    .body(json_body(&json!({
        "enabled": false, "provider": "OpenAI", "model": "m",
        "dimensions": 8, "include_name": true, "include_path": true,
    })))
    .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::FORBIDDEN);
}
