//! E3: package exports are an administrator operation, end to end over HTTP.
//!
//! `POST …/raisin:create-from-selection` and `GET …/raisin:download/{job}` used
//! to ignore the caller: an anonymous request could export any workspace, and
//! anyone holding a job id could download the archive — from any tenant. These
//! tests pin the refusal for every principal that is not an exporter, the
//! scoping of a download to the caller's own tenant, repository and package
//! jobs, and that an accepted build records who asked (the job then reads the
//! selection under that requester's row-level security; the filtering itself
//! is unit-tested in `raisin_core::services::package_export_access`).

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};

use raisin_core::services::package_export_access::{REQUESTER_AUTH_KEY, REQUESTER_PERMISSIONS_KEY};
use raisin_storage::jobs::{JobContext, JobId, JobType};

use crate::support::{admin, create_repository, fresh_storage, send, TENANT};

const REPO: &str = "pkgsec";

struct Fixture {
    app: axum::Router,
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    /// An identity user's access token: signed in, no admin role.
    user_token: String,
    /// An API key minted by an admin (resolves to the system context).
    api_key: String,
}

async fn fixture(label: &str) -> Fixture {
    let storage = fresh_storage(label);
    let auth = Arc::new(raisin_rocksdb::AuthService::new(
        raisin_rocksdb::AdminUserStore::new(storage.db().clone()),
        "http-package-export-auth-jwt-secret".to_string(),
    ));

    let identity = raisin_models::auth::Identity::new(
        "identity-alice".to_string(),
        TENANT.to_string(),
        "alice@example.com".to_string(),
    );
    let session = raisin_models::auth::Session::new(
        "session-alice".to_string(),
        TENANT.to_string(),
        "identity-alice".to_string(),
        "local".to_string(),
        "family-alice".to_string(),
        raisin_models::timestamp::StorageTimestamp::now(),
    );
    let user_token = auth
        .generate_user_tokens(&identity, &session, Some(REPO.to_string()), None)
        .unwrap()
        .access_token;

    let admin_user = auth
        .create_user(
            TENANT.to_string(),
            "pkg-admin".to_string(),
            None,
            "Pkg-Admin-Passw0rd!".to_string(),
            raisin_models::admin_user::AdminAccessFlags {
                console_login: true,
                cli_access: true,
                api_access: true,
                pgwire_access: true,
                can_impersonate: false,
            },
        )
        .unwrap();
    let api_key = auth
        .create_api_key(TENANT, &admin_user.user_id, "pkgsec")
        .unwrap()
        .1;

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
    Fixture {
        app,
        storage,
        user_token,
        api_key,
    }
}

fn bearer(req: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    req.header("authorization", format!("Bearer {token}"))
}

fn selection_body() -> Body {
    Body::from(
        serde_json::to_vec(&json!({
            "name": "leak",
            "version": "1.0.0",
            "selected_paths": [{ "workspace": "content", "path": "/*" }],
            "include_node_types": false,
        }))
        .unwrap(),
    )
}

fn create_request() -> axum::http::request::Builder {
    Request::builder()
        .method("POST")
        .uri(format!(
            "/api/packages/{REPO}/main/head/raisin:create-from-selection"
        ))
        .header("content-type", "application/json")
}

fn download_request(job_id: &str) -> axum::http::request::Builder {
    Request::builder().uri(format!(
        "/api/packages/{REPO}/main/head/leak/raisin:download/{job_id}"
    ))
}

/// Register a package job directly in the job system, as if some build had
/// been submitted, owned by `tenant` / `repo`.
async fn register_job(
    storage: &raisin_rocksdb::RocksDBStorage,
    tenant: &str,
    repo: &str,
    job_type: JobType,
) -> String {
    let job_id = JobId::new();
    storage
        .job_data_store()
        .put(
            &job_id,
            &JobContext {
                tenant_id: tenant.to_string(),
                repo_id: repo.to_string(),
                branch: "main".to_string(),
                workspace_id: "packages".to_string(),
                revision: raisin_hlc::HLC::now(),
                metadata: HashMap::new(),
            },
        )
        .unwrap();
    storage
        .job_registry()
        .register_job_with_id(
            job_id.clone(),
            job_type,
            tenant.to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    job_id.to_string()
}

fn package_job() -> JobType {
    JobType::PackageCreateFromSelection {
        package_name: "leak".to_string(),
        package_version: "1.0.0".to_string(),
        include_node_types: false,
    }
}

// ---- create-from-selection -------------------------------------------------

#[tokio::test]
async fn create_from_selection_refuses_anonymous_callers() {
    let f = fixture("pkg-create-anon").await;

    let (status, body) = send(&f.app, create_request().body(selection_body()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // A bearer that validates as nothing is anonymous too.
    let req = bearer(create_request(), "not-a-real-token")
        .body(selection_body())
        .unwrap();
    assert_eq!(send(&f.app, req).await.0, StatusCode::UNAUTHORIZED);

    // An API-key-shaped bearer that does not validate.
    let req = bearer(create_request(), "raisin_forged")
        .body(selection_body())
        .unwrap();
    assert_eq!(send(&f.app, req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_from_selection_refuses_a_signed_in_non_admin() {
    let f = fixture("pkg-create-user").await;
    let req = bearer(create_request(), &f.user_token)
        .body(selection_body())
        .unwrap();
    let (status, body) = send(&f.app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn refusal_comes_before_any_validation() {
    // An unauthorised caller must not be able to probe the request shape.
    let f = fixture("pkg-create-shape").await;
    let req = create_request()
        .body(Body::from(
            serde_json::to_vec(&json!({
                "name": "bad name!",
                "version": "1",
                "selected_paths": [],
                "include_node_types": false,
            }))
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(send(&f.app, req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_admin_build_records_who_asked() {
    let f = fixture("pkg-create-admin").await;

    for req in [
        admin(create_request()).body(selection_body()).unwrap(),
        bearer(create_request(), &f.api_key)
            .body(selection_body())
            .unwrap(),
    ] {
        let (status, body) = send(&f.app, req).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let response: Value = serde_json::from_str(&body).unwrap();
        let job_id = response["job_id"].as_str().unwrap().to_string();

        let context = f
            .storage
            .job_data_store()
            .get(TENANT, &JobId(job_id))
            .unwrap()
            .expect("job context stored");
        let requester = context
            .metadata
            .get(REQUESTER_AUTH_KEY)
            .expect("the build records its requester");
        assert_eq!(requester["is_system"], true);
        assert!(context.metadata.contains_key(REQUESTER_PERMISSIONS_KEY));
    }
}

// ---- download --------------------------------------------------------------

#[tokio::test]
async fn download_refuses_anonymous_and_non_admin_callers() {
    let f = fixture("pkg-download-auth").await;
    // A real package job of this tenant and repository: the refusal must not
    // depend on the job being missing.
    let job = register_job(&f.storage, TENANT, REPO, package_job()).await;

    let (status, body) = send(&f.app, download_request(&job).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let req = bearer(download_request(&job), "not-a-real-token")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&f.app, req).await.0, StatusCode::UNAUTHORIZED);

    let req = bearer(download_request(&job), &f.user_token)
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&f.app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn download_is_scoped_to_the_callers_tenant_repo_and_package_jobs() {
    let f = fixture("pkg-download-scope").await;

    let other_tenant = register_job(&f.storage, "other-tenant", REPO, package_job()).await;
    let other_repo = register_job(&f.storage, TENANT, "other-repo", package_job()).await;
    let not_a_package_job = register_job(&f.storage, TENANT, REPO, JobType::Compaction).await;

    for (label, job) in [
        ("another tenant's job", &other_tenant),
        ("another repository's job", &other_repo),
        ("a non-package job", &not_a_package_job),
        ("an unknown job", &"no-such-job".to_string()),
    ] {
        let req = admin(download_request(job)).body(Body::empty()).unwrap();
        let (status, body) = send(&f.app, req).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{label}: {body}");
    }

    // Our own package job passes the scoping; it is simply not finished.
    let own = register_job(&f.storage, TENANT, REPO, package_job()).await;
    let req = admin(download_request(&own)).body(Body::empty()).unwrap();
    let (status, body) = send(&f.app, req).await;
    assert!(
        ![
            StatusCode::NOT_FOUND,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::OK
        ]
        .contains(&status),
        "own unfinished job: {status} {body}"
    );
}

// ---- raisin:export -----------------------------------------------------------

#[tokio::test]
async fn export_refuses_anonymous_and_non_admin_callers() {
    let f = fixture("pkg-export-auth").await;
    let uri = format!("/api/packages/{REPO}/main/head/some-package/raisin:export");

    let req = Request::builder()
        .method("POST")
        .uri(&uri)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&f.app, req).await.0, StatusCode::UNAUTHORIZED);

    let req = bearer(Request::builder().method("POST").uri(&uri), &f.user_token)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&f.app, req).await.0, StatusCode::FORBIDDEN);
}
