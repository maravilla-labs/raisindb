//! Shared fixtures for the HTTP integration tests.
//!
//! Two things every test in this binary needs and used to get wrong on its own:
//!
//! 1. **A repository that exists.** Node types, workspaces and nodes are all
//!    scoped to a branch, and a branch belongs to a repository. Writing a node
//!    type into a branch nobody created fails with `Branch 'main' not found`
//!    before the test has sent a single request. [`Fixture::new`] creates the
//!    repository (and with it the default branch) through the same
//!    `POST /api/repositories` a client would call.
//!
//! 2. **A caller.** The repository and workspace surfaces run
//!    `optional_auth_middleware`, and an unauthenticated request is the
//!    anonymous principal — which may read nothing and write nothing unless a
//!    role says otherwise. Tests about node semantics (paths, moves, copies,
//!    revisions) authenticate as the operator superadmin, which
//!    `optional_auth_middleware` turns into `AuthContext::system()`.
//!
//! Environment variables are process-global and this is ONE test binary, so
//! there is exactly one superadmin token for all of it: two files setting two
//! different values would race, and whichever set it last would silently
//! de-authenticate the other file's requests.

#![cfg(not(feature = "s3"))]

use std::sync::{Arc, Once};

use axum::{
    body::Body,
    http::{HeaderValue, Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_models::nodes::types::NodeType;
use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository, Storage};

/// The storage the router is built over — the same choice `raisin_transport_http`
/// makes from its features.
#[cfg(feature = "storage-rocksdb")]
pub type TestStorage = raisin_rocksdb::RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
pub type TestStorage = raisin_storage_memory::InMemoryStorage;

/// The one bearer every test in this binary presents as the operator.
pub const ADMIN_TOKEN: &str = "http-integration-tests-superadmin-token";

/// A signing secret the tests also hold, so asset grants can be minted
/// in-process and presented to the router as a client would.
pub const SIGNING_SECRET: &str = "asset-grant-test-signing-secret-32b";

/// The tenant `ensure_tenant_middleware` resolves when no `x-tenant-id` is sent.
pub const TENANT: &str = "default";

static ENV: Once = Once::new();

/// Set the process-wide environment the router reads per request. Only ever
/// SET, never cleared, so tests running in parallel cannot race on it.
///
/// `RAISIN_CRYPTO_EMIT_V2` is required by the secret store: its crypto family
/// is `V1Policy::Reject`, so without it every write fails.
pub fn init_env() {
    ENV.call_once(|| {
        std::env::set_var("RAISIN_SUPERADMIN_TOKEN", ADMIN_TOKEN);
        std::env::set_var("RAISINDB_SIGNING_SECRET", SIGNING_SECRET);
        std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1");
    });
}

/// A fresh database, unique per call (a RocksDB under `/tmp`, or in memory).
#[cfg(feature = "storage-rocksdb")]
pub fn fresh_storage(label: &str) -> Arc<TestStorage> {
    init_env();
    let path = format!("/tmp/raisin-http-test-{label}-{}", nanoid::nanoid!(8));
    let _ = std::fs::remove_dir_all(&path);
    Arc::new(TestStorage::new(&path).expect("open RocksDB"))
}

/// A fresh database, unique per call (a RocksDB under `/tmp`, or in memory).
#[cfg(not(feature = "storage-rocksdb"))]
pub fn fresh_storage(_label: &str) -> Arc<TestStorage> {
    init_env();
    Arc::new(TestStorage::default())
}

/// Add the superadmin bearer to a request.
pub fn admin(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("authorization", format!("Bearer {ADMIN_TOKEN}"))
}

/// Wrap a router so that every request WITHOUT an `authorization` header is
/// sent as the operator superadmin. A request that carries its own credential
/// keeps it, so a test can still speak as someone else.
pub fn as_admin(router: axum::Router) -> axum::Router {
    init_env();
    router.layer(axum::middleware::map_request(
        |mut req: Request<Body>| async move {
            if !req.headers().contains_key("authorization") {
                req.headers_mut().insert(
                    "authorization",
                    HeaderValue::from_str(&format!("Bearer {ADMIN_TOKEN}")).unwrap(),
                );
            }
            req
        },
    ))
}

/// Send a request and return its status and body text.
pub async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Create repository `repo` (and its default branch `branch`) as the operator.
pub async fn create_repository(app: &axum::Router, repo: &str, branch: &str) {
    let body = serde_json::json!({
        "repo_id": repo,
        "description": "http integration test repository",
        "default_branch": branch,
    });
    let req = admin(
        Request::builder()
            .method("POST")
            .uri("/api/repositories")
            .header("content-type", "application/json"),
    )
    .body(Body::from(serde_json::to_vec(&body).unwrap()))
    .unwrap();
    let (status, text) = send(app, req).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::CONFLICT,
        "creating repository {repo}: {status} {text}"
    );
}

/// Create (or replace) workspace `ws` in `repo` as the operator.
pub async fn create_workspace(app: &axum::Router, repo: &str, ws: &str) {
    let body = serde_json::json!({
        "name": ws,
        "allowed_node_types": [],
        "allowed_root_node_types": [],
        "depends_on": [],
    });
    let req = admin(
        Request::builder()
            .method("PUT")
            .uri(format!("/api/workspaces/{repo}/{ws}"))
            .header("content-type", "application/json"),
    )
    .body(Body::from(serde_json::to_vec(&body).unwrap()))
    .unwrap();
    let (status, text) = send(app, req).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "creating workspace {repo}/{ws}: {text}"
    );
}

/// Register a permissive (non-strict) node type named `name` on `repo/branch`.
pub async fn register_node_type(storage: &TestStorage, repo: &str, branch: &str, name: &str) {
    let node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: vec![],
        overrides: None,
        description: Some(format!("Test NodeType: {name}")),
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec![],
        required_nodes: vec![],
        initial_structure: None,
        versionable: Some(true),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: None,
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        compound_indexes: None,
        is_mixin: None,
        previous_version: None,
    };
    storage
        .node_types()
        .put(
            BranchScope::new(TENANT, repo, branch),
            node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

/// A router over a fresh database holding one repository, its default branch,
/// the listed workspaces and the listed node types.
pub struct Fixture {
    /// The router, which sends unauthenticated requests as the operator.
    pub app: axum::Router,
    /// The same router WITHOUT the operator default, for anonymous requests.
    pub raw: axum::Router,
    pub storage: Arc<TestStorage>,
}

impl Fixture {
    pub async fn new(
        label: &str,
        repo: &str,
        branch: &str,
        workspaces: &[&str],
        node_types: &[&str],
    ) -> Self {
        let storage = fresh_storage(label);
        let raw = raisin_transport_http::router(storage.clone());
        create_repository(&raw, repo, branch).await;
        for name in node_types {
            register_node_type(&storage, repo, branch, name).await;
        }
        for ws in workspaces {
            create_workspace(&raw, repo, ws).await;
        }
        Self {
            app: as_admin(raw.clone()),
            raw,
            storage,
        }
    }
}

/// Create the node at `path` through `POST` on its parent — the create verb.
///
/// `base` is the workspace's HEAD route, e.g. `/api/repository/r/main/head/ws`.
/// `PUT` on a node path UPDATES an existing node; it never created one, so a
/// test that seeds with it seeds nothing.
pub async fn create_at(
    app: &axum::Router,
    base: &str,
    id: &str,
    name: &str,
    path: &str,
    node_type: &str,
) -> (StatusCode, String) {
    let parent = match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    };
    let uri = if parent == "/" {
        format!("{base}/")
    } else {
        format!("{base}{parent}")
    };
    let body = serde_json::json!({
        "id": id,
        "name": name,
        "path": path,
        "node_type": node_type,
        "properties": {},
    });
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    send(app, req).await
}
