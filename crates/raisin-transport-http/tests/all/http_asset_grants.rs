//! `?grant=…` on the asset routes — the scoped alternative to a per-asset
//! signature.
//!
//! The unit tests in `raisin_core::asset_grants` prove the token itself: what it
//! covers, what it refuses, and that tampering fails closed. What can only be
//! proved HERE is that the serve route is wired to that verifier and to nothing
//! else — that a grant reaching the HTTP surface is checked for scope, for
//! expiry, and is read under the named subject's row-level security rather than
//! around it.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::sync::{Arc, Once};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_models::nodes::types::NodeType;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository, Storage};

/// The bearer `optional_auth_middleware` turns into `AuthContext::system()`.
/// Used here to set fixtures up, and to prove that a system caller is exactly
/// the principal a grant may NOT be minted for.
const ADMIN_TOKEN: &str = "asset-grants-test-superadmin-token";

/// A signing secret the test also holds, so a grant can be minted in-process and
/// presented to the router as a client would.
const SIGNING_SECRET: &str = "asset-grant-test-signing-secret-32b";

/// Env vars are process-global. Both are only ever SET, never cleared, so tests
/// running in parallel in this binary cannot race on them.
static ENV: Once = Once::new();

fn init_env() {
    ENV.call_once(|| {
        std::env::set_var("RAISIN_SUPERADMIN_TOKEN", ADMIN_TOKEN);
        std::env::set_var("RAISINDB_SIGNING_SECRET", SIGNING_SECRET);
    });
}

const TENANT: &str = "default";
const REPO: &str = "main";
const BRANCH: &str = "main";
const WORKSPACE: &str = "demo";

async fn register_node_type(storage: &RocksDBStorage, name: &str) {
    let node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: vec![],
        overrides: None,
        description: None,
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
            BranchScope::new(TENANT, REPO, BRANCH),
            node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

async fn app(name: &str) -> axum::Router {
    app_with_storage(name).await.0
}

async fn app_with_storage(name: &str) -> (axum::Router, Arc<RocksDBStorage>) {
    init_env();
    let path = format!("/tmp/raisin-asset-grants-test-{name}");
    let _ = std::fs::remove_dir_all(&path);
    let store = Arc::new(RocksDBStorage::new(&path).unwrap());
    let router = raisin_transport_http::router(store.clone());

    let repo_body = serde_json::json!({
        "repo_id": REPO,
        "description": "asset grant tests",
        "default_branch": BRANCH
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repositories")
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&repo_body).unwrap()))
        .unwrap();
    let (status, body) = send(&router, req).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::CONFLICT,
        "{status} {body}"
    );

    let ws_body = serde_json::json!({
        "name": WORKSPACE,
        "allowed_node_types": [],
        "allowed_root_node_types": [],
        "depends_on": []
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/workspaces/{BRANCH}/{WORKSPACE}"))
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let (status, body) = send(&router, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    register_node_type(&store, "asset").await;

    (router, store)
}

/// Seed the access-control workspace with a role that may read everything and a
/// user holding it, so a grant naming that user resolves to real permissions.
async fn grant_read_to(router: &axum::Router, store: &Arc<RocksDBStorage>, subject: &str) {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    let ws_body = serde_json::json!({
        "name": "raisin:access_control",
        "allowed_node_types": [],
        "allowed_root_node_types": [],
        "depends_on": []
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/workspaces/{REPO}/raisin:access_control"))
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let (status, body) = send(router, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let node = |id: &str,
                path: &str,
                node_type: &str,
                parent: &str,
                props: HashMap<String, PropertyValue>| {
        raisin_models::nodes::Node {
            id: id.to_string(),
            name: id.to_string(),
            path: path.to_string(),
            node_type: node_type.to_string(),
            archetype: None,
            properties: props,
            children: vec![],
            order_key: String::new(),
            has_children: None,
            parent: Some(parent.to_string()),
            version: 1,
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            published_at: None,
            published_by: None,
            updated_by: Some("test".to_string()),
            created_by: Some("test".to_string()),
            translations: None,
            tenant_id: Some(TENANT.to_string()),
            workspace: Some("raisin:access_control".to_string()),
            owner_id: None,
            relations: Vec::new(),
        }
    };

    let nodes = store.nodes_impl();
    for folder in ["users", "roles"] {
        nodes
            .add(
                TENANT,
                REPO,
                BRANCH,
                "raisin:access_control",
                node(
                    folder,
                    &format!("/{folder}"),
                    "raisin:AclFolder",
                    "/",
                    HashMap::new(),
                ),
            )
            .await
            .unwrap();
    }

    let mut role_props = HashMap::new();
    role_props.insert(
        "role_id".to_string(),
        PropertyValue::String("reader".into()),
    );
    role_props.insert("name".to_string(), PropertyValue::String("reader".into()));
    role_props.insert("inherits".to_string(), PropertyValue::Array(vec![]));
    let mut permission = HashMap::new();
    permission.insert("path".to_string(), PropertyValue::String("**".into()));
    permission.insert(
        "operations".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("read".into())]),
    );
    role_props.insert(
        "permissions".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(permission)]),
    );
    nodes
        .add(
            TENANT,
            REPO,
            BRANCH,
            "raisin:access_control",
            node(
                "reader",
                "/roles/reader",
                "raisin:Role",
                "roles",
                role_props,
            ),
        )
        .await
        .unwrap();

    let mut user_props = HashMap::new();
    user_props.insert(
        "user_id".to_string(),
        PropertyValue::String(subject.to_string()),
    );
    user_props.insert(
        "roles".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("reader".into())]),
    );
    nodes
        .add(
            TENANT,
            REPO,
            BRANCH,
            "raisin:access_control",
            node(
                "reader-user",
                "/users/reader-user",
                "raisin:User",
                "users",
                user_props,
            ),
        )
        .await
        .unwrap();
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Create a node at `parent` named `name`, as the system caller.
async fn create_node(app: &axum::Router, parent: &str, name: &str) {
    let body = serde_json::json!({
        "name": name,
        "node_type": "asset",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}{parent}"
        ))
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, body) = send(app, req).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "creating {parent}{name} failed: {status} {body}"
    );
}

fn grant_for(prefix: &str, subject: &str, expires: u64) -> String {
    raisin_core::mint_asset_grant(
        SIGNING_SECRET.as_bytes(),
        &raisin_core::AssetGrant {
            tenant_id: TENANT.to_string(),
            repo: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace: WORKSPACE.to_string(),
            prefix: prefix.to_string(),
            subject: subject.to_string(),
            email: None,
            home: None,
            expires,
        },
    )
}

fn far_future() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}

async fn display(app: &axum::Router, path: &str, query: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .uri(format!(
            "/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}{path}/raisin:display?{query}"
        ))
        .body(Body::empty())
        .unwrap();
    send(app, req).await
}

// ---- minting ------------------------------------------------------------

/// A system principal bypasses row-level security, so a grant naming one would
/// be a standing key to a subtree with no filtering behind it. Refused — the
/// per-asset signature is the instrument for that caller.
#[tokio::test]
async fn a_system_caller_cannot_mint_a_grant() {
    let app = app("system-mint").await;

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/raisin:grant"
        ))
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("GRANT_REQUIRES_USER"), "{body}");
    assert!(!body.contains("rag1."), "no token may be issued: {body}");
}

/// A grant bound to nobody is a plain bearer token. Refused at the mint.
#[tokio::test]
async fn an_unauthenticated_caller_cannot_mint_a_grant() {
    let app = app("anon-mint").await;

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/raisin:grant"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "expected a refusal, got {status}: {body}"
    );
    assert!(!body.contains("rag1."), "no token may be issued: {body}");
}

// ---- serving ------------------------------------------------------------

/// A grant is refused for a sibling whose NAME merely starts with the granted
/// one. This is the segment-boundary rule, asserted end to end.
#[tokio::test]
async fn a_sibling_prefix_is_refused() {
    let app = app("sibling").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/", "photos-private").await;
    create_node(&app, "/photos-private/", "a.jpg").await;

    let token = grant_for("/photos", "user-1", far_future());
    let (status, body) = display(&app, "/photos-private/a.jpg", &format!("grant={token}")).await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("GRANT_OUT_OF_SCOPE"), "{body}");
}

/// The clock is enforced at the serve route, not only in the token.
#[tokio::test]
async fn an_expired_grant_is_refused() {
    let app = app("expired").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;

    let token = grant_for("/photos", "user-1", 1);
    let (status, body) = display(&app, "/photos/a.jpg", &format!("grant={token}")).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("CREDENTIAL_EXPIRED"), "{body}");
}

/// A grant signed with a secret this deployment does not hold, a grant whose
/// payload was widened after signing, and a string that is not a token at all.
#[tokio::test]
async fn a_tampered_grant_is_refused() {
    let app = app("tampered").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;

    let foreign = raisin_core::mint_asset_grant(
        b"a-different-secret-nobody-here-has",
        &raisin_core::AssetGrant {
            tenant_id: TENANT.to_string(),
            repo: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace: WORKSPACE.to_string(),
            prefix: "/".to_string(),
            subject: "user-1".to_string(),
            email: None,
            home: None,
            expires: far_future(),
        },
    );

    let legitimate = grant_for("/photos", "user-1", far_future());
    let mut parts = legitimate.split('.');
    let (version, _payload, signature) = (
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap(),
    );
    // The same grant widened to the whole workspace, carrying the old signature.
    let widened = grant_for("/", "user-1", far_future());
    let widened_payload = widened.split('.').nth(1).unwrap();
    let forged = format!("{version}.{widened_payload}.{signature}");

    for token in [foreign, forged, "rag1.nonsense".to_string()] {
        let (status, body) = display(&app, "/photos/a.jpg", &format!("grant={token}")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token}: {body}");
    }
}

/// A grant cannot reach into another workspace, even for the same path.
#[tokio::test]
async fn a_grant_cannot_cross_a_workspace() {
    let app = app("cross-workspace").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;

    let token = raisin_core::mint_asset_grant(
        SIGNING_SECRET.as_bytes(),
        &raisin_core::AssetGrant {
            tenant_id: TENANT.to_string(),
            repo: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace: "somewhere-else".to_string(),
            prefix: "/photos".to_string(),
            subject: "user-1".to_string(),
            email: None,
            home: None,
            expires: far_future(),
        },
    );

    let (status, body) = display(&app, "/photos/a.jpg", &format!("grant={token}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("GRANT_OUT_OF_SCOPE"), "{body}");
}

/// The property the whole design rests on: a grant confers no authority of its
/// own. Here the token is valid, live and in scope, and it still returns
/// nothing, because its SUBJECT has no permissions in this repository and the
/// read is performed as them.
///
/// The per-asset signature on the same asset is the control: it IS the
/// authority, so it reaches the node and fails on the missing binary instead.
/// Two different failures from one route is what proves the two forms take
/// different paths to the data.
#[tokio::test]
async fn a_grant_never_exceeds_its_subject() {
    let app = app("rls").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;

    let token = grant_for("/photos", "a-user-with-no-permissions", far_future());
    let (status, body) = display(&app, "/photos/a.jpg", &format!("grant={token}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body.contains("Node not found"),
        "a node the subject may not read must read as absent: {body}"
    );

    let expires = far_future();
    let signed = raisin_core::build_signed_asset_url(
        SIGNING_SECRET.as_bytes(),
        TENANT,
        REPO,
        BRANCH,
        WORKSPACE,
        "/photos/a.jpg",
        "file",
        "display",
        expires,
        None,
    );
    let sig = signed
        .url
        .split("sig=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .unwrap()
        .to_string();
    let (status, body) = display(&app, "/photos/a.jpg", &format!("sig={sig}&exp={expires}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body.contains("property"),
        "the signature must reach the node itself: {body}"
    );
}

/// The historical form keeps working, unchanged, through the shared verifier —
/// including its refusal of a signature minted for a different asset.
#[tokio::test]
async fn a_signature_still_opens_only_its_own_asset() {
    let app = app("signature").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;
    create_node(&app, "/photos/", "b.jpg").await;

    let expires = far_future();
    let signed = raisin_core::build_signed_asset_url(
        SIGNING_SECRET.as_bytes(),
        TENANT,
        REPO,
        BRANCH,
        WORKSPACE,
        "/photos/a.jpg",
        "file",
        "display",
        expires,
        None,
    );
    let sig = signed
        .url
        .split("sig=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .unwrap()
        .to_string();

    let (status, body) = display(&app, "/photos/b.jpg", &format!("sig={sig}&exp={expires}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("INVALID_SIGNATURE"), "{body}");
}

/// No credential at all is a refusal, not a read.
#[tokio::test]
async fn an_asset_read_with_no_credential_is_refused() {
    let app = app("no-credential").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;

    let (status, body) = display(&app, "/photos/a.jpg", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

/// The positive half of the same property: a grant whose subject MAY read the
/// subtree reaches the node, and one asset after another does so on the same
/// token — which is the point of minting one grant per page rather than one
/// signature per thumbnail.
///
/// "Reaches the node" is asserted as the missing-binary error: these fixtures
/// carry no stored file, so getting that far is precisely the proof that the
/// row-level security check let the read through.
#[tokio::test]
async fn a_grant_opens_every_asset_its_subject_may_read() {
    let (app, store) = app_with_storage("permitted").await;
    create_node(&app, "/", "photos").await;
    create_node(&app, "/photos/", "a.jpg").await;
    create_node(&app, "/photos/", "b.jpg").await;
    grant_read_to(&app, &store, "permitted-user").await;

    let token = grant_for("/photos", "permitted-user", far_future());

    for asset in ["/photos/a.jpg", "/photos/b.jpg"] {
        let (status, body) = display(&app, asset, &format!("grant={token}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{asset}: {body}");
        assert!(
            body.contains("property"),
            "{asset} should have been reached, not filtered away: {body}"
        );
    }
}
