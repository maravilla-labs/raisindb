//! `raisin:display` / `raisin:download` WITHOUT a credential: the request is
//! read as its own principal under row-level security.
//!
//! A site whose anonymous role may read published assets (`branch_pattern:
//! "publish"`) links the bytes directly. What is proved here, over the real
//! router:
//!
//! - an anonymous caller gets a published asset: inline display, range
//!   requests (206), and another Resource property of the same node;
//! - the SAME asset on a branch the role does not cover, and a workspace it
//!   does not cover, answer 404 — never the bytes, and the same answer as a
//!   node that does not exist;
//! - a role without grants (and anonymous access switched off) keeps the old
//!   401;
//! - a signed link works as before, and a wrong or expired signature is
//!   refused and never falls back to the unsigned read.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::NodeType;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository, Storage};

use crate::support::{init_env, ADMIN_TOKEN, SIGNING_SECRET};

const TENANT: &str = "default";
const REPO: &str = "pubrepo";
const MAIN: &str = "main";
const PUBLISH: &str = "publish";
const WORKSPACE: &str = "media";
const OTHER_WS: &str = "private";
const BYTES: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

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

fn admin(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("authorization", format!("Bearer {ADMIN_TOKEN}"))
}

async fn register_node_type(storage: &RocksDBStorage, branch: &str, name: &str) {
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
            BranchScope::new(TENANT, REPO, branch),
            node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

fn node(
    id: &str,
    path: &str,
    node_type: &str,
    parent: &str,
    workspace: &str,
    props: HashMap<String, PropertyValue>,
) -> raisin_models::nodes::Node {
    raisin_models::nodes::Node {
        id: id.to_string(),
        name: path.rsplit('/').next().unwrap_or(id).to_string(),
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
        workspace: Some(workspace.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

async fn put_workspace(app: &axum::Router, name: &str) {
    let body = serde_json::json!({
        "name": name, "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []
    });
    let res = send(
        app,
        admin(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/workspaces/{REPO}/{name}")),
        )
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap(),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&res.body)
    );
}

/// Anonymous access on for the repository, and the anonymous user holding a
/// role with exactly `permissions`.
async fn anonymous_role(
    app: &axum::Router,
    store: &Arc<RocksDBStorage>,
    permissions: Vec<PropertyValue>,
) {
    put_workspace(app, "raisin:system").await;
    put_workspace(app, "raisin:access_control").await;
    let nodes = store.nodes_impl();
    let add = |ws: &'static str, n: raisin_models::nodes::Node| {
        let nodes = nodes.clone();
        async move { nodes.add(TENANT, REPO, MAIN, ws, n).await.unwrap() }
    };
    add(
        "raisin:system",
        node(
            "config",
            "/config",
            "raisin:Folder",
            "/",
            "raisin:system",
            HashMap::new(),
        ),
    )
    .await;
    add(
        "raisin:system",
        node(
            "repos",
            "/config/repos",
            "raisin:Folder",
            "config",
            "raisin:system",
            HashMap::new(),
        ),
    )
    .await;
    let mut cfg = HashMap::new();
    cfg.insert(
        "anonymous_enabled".to_string(),
        PropertyValue::Boolean(true),
    );
    add(
        "raisin:system",
        node(
            REPO,
            &format!("/config/repos/{REPO}"),
            "raisin:RepoAuthConfig",
            "repos",
            "raisin:system",
            cfg,
        ),
    )
    .await;

    for folder in ["users", "roles"] {
        add(
            "raisin:access_control",
            node(
                folder,
                &format!("/{folder}"),
                "raisin:AclFolder",
                "/",
                "raisin:access_control",
                HashMap::new(),
            ),
        )
        .await;
    }
    let mut role = HashMap::new();
    role.insert(
        "role_id".to_string(),
        PropertyValue::String("anonymous".into()),
    );
    role.insert(
        "name".to_string(),
        PropertyValue::String("anonymous".into()),
    );
    role.insert("inherits".to_string(), PropertyValue::Array(vec![]));
    role.insert("permissions".to_string(), PropertyValue::Array(permissions));
    add(
        "raisin:access_control",
        node(
            "anonymous-role",
            "/roles/anonymous",
            "raisin:Role",
            "roles",
            "raisin:access_control",
            role,
        ),
    )
    .await;
    let mut user = HashMap::new();
    user.insert(
        "user_id".to_string(),
        PropertyValue::String("anonymous".into()),
    );
    user.insert(
        "roles".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("anonymous".into())]),
    );
    add(
        "raisin:access_control",
        node(
            "anonymous-user",
            "/users/anonymous",
            "raisin:User",
            "users",
            "raisin:access_control",
            user,
        ),
    )
    .await;
}

fn read_permission(workspace: &str, branch_pattern: Option<&str>) -> PropertyValue {
    let mut p = HashMap::new();
    p.insert(
        "workspace".to_string(),
        PropertyValue::String(workspace.into()),
    );
    p.insert("path".to_string(), PropertyValue::String("/**".into()));
    p.insert(
        "operations".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("read".into())]),
    );
    if let Some(b) = branch_pattern {
        p.insert(
            "branch_pattern".to_string(),
            PropertyValue::String(b.into()),
        );
    }
    PropertyValue::Object(p)
}

/// Create `/photos/<name>` in `ws` on `branch` and upload BYTES as its `file`.
async fn asset_with_bytes(app: &axum::Router, branch: &str, ws: &str, name: &str) {
    let base = format!("/api/repository/{REPO}/{branch}/head/{ws}");
    let body = serde_json::json!({ "name": name, "node_type": "asset", "properties": {} });
    let res = send(
        app,
        admin(
            Request::builder()
                .method("POST")
                .uri(format!("{base}/photos?deep=true")),
        )
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap(),
    )
    .await;
    assert!(
        res.status.is_success(),
        "create: {} {}",
        res.status,
        String::from_utf8_lossy(&res.body)
    );

    let boundary = "XBOUNDARY";
    let mut buf: Vec<u8> = Vec::new();
    use std::io::Write;
    write!(buf, "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").unwrap();
    buf.extend_from_slice(BYTES);
    write!(buf, "\r\n--{boundary}--\r\n").unwrap();
    let res = send(
        app,
        admin(
            Request::builder()
                .method("POST")
                .uri(format!("{base}/photos/{name}")),
        )
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
        "upload: {}",
        String::from_utf8_lossy(&res.body)
    );
}

async fn create_branch(app: &axum::Router, name: &str) {
    let body = serde_json::json!({ "name": name, "from_revision": null, "created_by": "test", "protected": false });
    let res = send(
        app,
        admin(Request::builder().method("POST").uri(format!(
            "/api/management/repositories/{TENANT}/{REPO}/branches"
        )))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap(),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::CREATED,
        "branch: {}",
        String::from_utf8_lossy(&res.body)
    );
}

/// A repository with `media` (published on `publish`) and `private`, both
/// holding `/photos/a.bin` with BYTES, and an anonymous role from `permissions`.
async fn fixture(label: &str, permissions: Vec<PropertyValue>) -> axum::Router {
    init_env();
    let path = format!("/tmp/raisin-asset-public-test-{label}");
    let _ = std::fs::remove_dir_all(&path);
    let store = Arc::new(RocksDBStorage::new(&path).unwrap());
    let app = raisin_transport_http::router(store.clone());

    let body = serde_json::json!({ "repo_id": REPO, "description": "public assets", "default_branch": MAIN });
    let res = send(
        &app,
        admin(Request::builder().method("POST").uri("/api/repositories"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await;
    assert!(res.status == StatusCode::CREATED || res.status == StatusCode::CONFLICT);
    put_workspace(&app, WORKSPACE).await;
    put_workspace(&app, OTHER_WS).await;
    anonymous_role(&app, &store, permissions).await;
    // `publish` as Studio creates it: empty, then the published nodes copied in.
    // Here the same asset is written to both branches directly.
    create_branch(&app, PUBLISH).await;
    for branch in [MAIN, PUBLISH] {
        register_node_type(&store, branch, "asset").await;
        register_node_type(&store, branch, "raisin:Folder").await;
        asset_with_bytes(&app, branch, WORKSPACE, "a.bin").await;
        asset_with_bytes(&app, branch, OTHER_WS, "a.bin").await;
    }
    app
}

/// An unauthenticated GET.
async fn anon(app: &axum::Router, branch: &str, ws: &str, rest: &str, range: Option<&str>) -> Res {
    let mut req =
        Request::builder().uri(format!("/api/repository/{REPO}/{branch}/head/{ws}{rest}"));
    if let Some(r) = range {
        req = req.header("range", r);
    }
    send(app, req.body(Body::empty()).unwrap()).await
}

fn published_only() -> Vec<PropertyValue> {
    vec![read_permission(WORKSPACE, Some(PUBLISH))]
}

#[tokio::test]
async fn an_anonymous_caller_reads_a_published_asset_inline_with_ranges() {
    let app = fixture("allowed", published_only()).await;

    let full = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        "/photos/a.bin/raisin:display",
        None,
    )
    .await;
    assert_eq!(
        full.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&full.body)
    );
    assert_eq!(full.body, BYTES);
    assert_eq!(full.headers.get("content-disposition").unwrap(), "inline");
    assert_eq!(full.headers.get("accept-ranges").unwrap(), "bytes");

    let part = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        "/photos/a.bin/raisin:display",
        Some("bytes=10-19"),
    )
    .await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body, &BYTES[10..20]);
    assert_eq!(
        part.headers.get("content-range").unwrap(),
        &format!("bytes 10-19/{}", BYTES.len())
    );

    // the same node's file through the explicit property path, as a download
    let dl = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        "/photos/a.bin@file/raisin:download",
        None,
    )
    .await;
    assert_eq!(dl.status, StatusCode::OK);
    assert_eq!(dl.body, BYTES);
    assert!(dl
        .headers
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("attachment"));
}

#[tokio::test]
async fn outside_the_role_the_answer_is_404_and_never_the_bytes() {
    let app = fixture("denied", published_only()).await;

    // the SAME asset on main: the role covers `publish` only
    let main = anon(&app, MAIN, WORKSPACE, "/photos/a.bin/raisin:display", None).await;
    assert_eq!(
        main.status,
        StatusCode::NOT_FOUND,
        "{}",
        String::from_utf8_lossy(&main.body)
    );
    assert_ne!(main.body, BYTES);
    // a workspace the role does not cover, on the covered branch
    let other = anon(
        &app,
        PUBLISH,
        OTHER_WS,
        "/photos/a.bin/raisin:display",
        Some("bytes=0-3"),
    )
    .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    assert_ne!(other.body, &BYTES[0..4]);
    // indistinguishable from a node that does not exist
    let missing = anon(
        &app,
        PUBLISH,
        OTHER_WS,
        "/photos/nope.bin/raisin:display",
        None,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let shape = |b: &[u8]| {
        let v: serde_json::Value = serde_json::from_slice(b).unwrap();
        (v["code"].clone(), v["message"].clone())
    };
    assert_eq!(shape(&other.body), shape(&missing.body));
}

#[tokio::test]
async fn a_role_without_grants_keeps_the_old_401() {
    let app = fixture("no-grants", vec![]).await;
    let res = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        "/photos/a.bin/raisin:display",
        None,
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        String::from_utf8_lossy(&res.body)
    );
}

fn signed_query(branch: &str, ws: &str, path: &str, expires: u64) -> String {
    let signed = raisin_core::build_signed_asset_url(
        SIGNING_SECRET.as_bytes(),
        TENANT,
        REPO,
        branch,
        ws,
        path,
        "file",
        "display",
        expires,
        None,
    );
    signed
        .url
        .split_once('?')
        .map(|(_, q)| q.to_string())
        .unwrap()
}

#[tokio::test]
async fn a_signed_link_works_and_a_bad_signature_never_falls_back() {
    let app = fixture("signed", published_only()).await;
    let future = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    // a signature opens what the role does not: main, as before
    let q = signed_query(MAIN, WORKSPACE, "/photos/a.bin", future);
    let ok = anon(
        &app,
        MAIN,
        WORKSPACE,
        &format!("/photos/a.bin/raisin:display?{q}"),
        None,
    )
    .await;
    assert_eq!(
        ok.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&ok.body)
    );
    assert_eq!(ok.body, BYTES);

    // a wrong signature on a PUBLIC asset is still refused: presenting a
    // credential means it must verify
    let bad = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        "/photos/a.bin/raisin:display?sig=AAAA&exp=99999999999",
        None,
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    assert_ne!(bad.body, BYTES);

    // an expired signature too
    let q = signed_query(PUBLISH, WORKSPACE, "/photos/a.bin", 1);
    let expired = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        &format!("/photos/a.bin/raisin:display?{q}"),
        None,
    )
    .await;
    assert_eq!(expired.status, StatusCode::UNAUTHORIZED);
    assert_ne!(expired.body, BYTES);

    // a garbage grant as well
    let grant = anon(
        &app,
        PUBLISH,
        WORKSPACE,
        "/photos/a.bin/raisin:display?grant=nonsense",
        None,
    )
    .await;
    assert!(grant.status == StatusCode::UNAUTHORIZED || grant.status == StatusCode::FORBIDDEN);
    assert_ne!(grant.body, BYTES);
}
