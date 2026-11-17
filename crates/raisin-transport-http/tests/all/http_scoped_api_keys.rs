//! Scoped API keys: a key that carries `{ repo, roles }` resolves to the RLS
//! context of exactly those roles in exactly that repository. It is never the
//! system context, works in no other repository and on no management route.
//! An admin's own (unscoped) key keeps resolving to the system context.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use raisin_models::api_key::ApiKeyScope;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::RocksDBStorage;

use crate::support::{
    create_at, create_repository, create_workspace, fresh_storage, register_node_type, send, TENANT,
};

const REPO: &str = "site";
const OTHER: &str = "elsewhere";
const ACL: &str = "raisin:access_control";

struct Env {
    app: axum::Router,
    storage: Arc<RocksDBStorage>,
    auth: Arc<raisin_rocksdb::AuthService>,
    /// An admin's own key: unscoped, system context.
    admin_key: String,
}

fn acl_node(
    id: &str,
    path: &str,
    node_type: &str,
    parent: &str,
    props: HashMap<String, PropertyValue>,
) -> raisin_models::nodes::Node {
    raisin_models::nodes::Node {
        id: id.to_string(),
        name: id.to_string(),
        path: path.to_string(),
        node_type: node_type.to_string(),
        properties: props,
        parent: Some(parent.to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        updated_by: Some("test".to_string()),
        created_by: Some("test".to_string()),
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(ACL.to_string()),
        ..Default::default()
    }
}

fn role(id: &str, workspace: &str, operations: &[&str]) -> HashMap<String, PropertyValue> {
    let mut permission = HashMap::new();
    permission.insert("path".to_string(), PropertyValue::String("**".into()));
    permission.insert(
        "workspace".to_string(),
        PropertyValue::String(workspace.into()),
    );
    permission.insert(
        "operations".to_string(),
        PropertyValue::Array(
            operations
                .iter()
                .map(|o| PropertyValue::String(o.to_string()))
                .collect(),
        ),
    );
    HashMap::from([
        ("role_id".to_string(), PropertyValue::String(id.into())),
        ("name".to_string(), PropertyValue::String(id.into())),
        ("inherits".to_string(), PropertyValue::Array(vec![])),
        (
            "permissions".to_string(),
            PropertyValue::Array(vec![PropertyValue::Object(permission)]),
        ),
    ])
}

async fn env(label: &str) -> Env {
    let storage = fresh_storage(label);
    let auth = Arc::new(raisin_rocksdb::AuthService::new(
        raisin_rocksdb::AdminUserStore::new(storage.db().clone()),
        "http-scoped-keys-jwt-secret".to_string(),
    ));
    let user = auth
        .create_user(
            TENANT.to_string(),
            "key-admin".to_string(),
            None,
            "Key-Admin-Passw0rd!".to_string(),
            raisin_models::admin_user::AdminAccessFlags {
                console_login: true,
                cli_access: true,
                api_access: true,
                pgwire_access: true,
                can_impersonate: false,
            },
        )
        .unwrap();
    let admin_key = auth
        .create_api_key(TENANT, &user.user_id, "admin-own")
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
        Some(auth.clone()),
        None,
        None,
        None,
    );

    for repo in [REPO, OTHER] {
        create_repository(&app, repo, "main").await;
        register_node_type(&storage, repo, "main", "test:Doc").await;
        for ws in ["stories", "secrets", ACL] {
            create_workspace(&app, repo, ws).await;
        }
        for (ws, name) in [("stories", "public-page"), ("secrets", "private-note")] {
            let base = format!("/api/repository/{repo}/main/head/{ws}");
            let (status, text) = send_create(&app, &base, name).await;
            assert!(
                status == StatusCode::CREATED || status == StatusCode::OK,
                "seed {repo}/{ws}/{name}: {status} {text}"
            );
        }
    }

    // Roles live in the site repository's ACL workspace only.
    let nodes = storage.nodes_impl();
    nodes
        .add(
            TENANT,
            REPO,
            "main",
            ACL,
            acl_node("roles", "/roles", "raisin:AclFolder", "/", HashMap::new()),
        )
        .await
        .unwrap();
    for (id, props) in [
        ("site_reader", role("site_reader", "stories", &["read"])),
        ("acl_writer", role("acl_writer", ACL, &["create", "update"])),
    ] {
        nodes
            .add(
                TENANT,
                REPO,
                "main",
                ACL,
                acl_node(id, &format!("/roles/{id}"), "raisin:Role", "roles", props),
            )
            .await
            .unwrap();
    }

    Env {
        app,
        storage,
        auth,
        admin_key,
    }
}

async fn send_create(app: &axum::Router, base: &str, name: &str) -> (StatusCode, String) {
    // `create_at` sends no credential of its own; this router has no
    // superadmin default, so wrap it.
    let app = crate::support::as_admin(app.clone());
    create_at(&app, base, name, name, &format!("/{name}"), "test:Doc").await
}

fn get(uri: &str, bearer: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::empty())
        .unwrap()
}

fn mint(env: &Env, roles: &[&str]) -> (String, String) {
    let (key, token) = env
        .auth
        .create_scoped_api_key(
            TENANT,
            "site:test",
            ApiKeyScope {
                repo: REPO.to_string(),
                roles: roles.iter().map(|r| r.to_string()).collect(),
            },
        )
        .unwrap();
    (key.key_id, token)
}

fn node_uri(repo: &str, ws: &str, name: &str) -> String {
    format!("/api/repository/{repo}/main/head/{ws}/{name}")
}

#[tokio::test]
async fn a_scoped_key_reads_what_its_roles_allow_and_nothing_else() {
    let env = env("scoped-reads").await;
    let (_, token) = mint(&env, &["site_reader"]);

    let (status, text) = send(
        &env.app,
        get(&node_uri(REPO, "stories", "public-page"), &token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "its role reads stories: {text}");

    let (status, text) = send(
        &env.app,
        get(&node_uri(REPO, "secrets", "private-note"), &token),
    )
    .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "no role reads secrets: {status} {text}"
    );
    // The refusal names the path asked for, never the node's content.
    assert!(!text.contains("\"node_type\""), "{text}");
}

#[tokio::test]
async fn a_scoped_key_is_refused_in_another_repository() {
    let env = env("scoped-other-repo").await;
    let (_, token) = mint(&env, &["site_reader"]);
    let (status, text) = send(
        &env.app,
        get(&node_uri(OTHER, "stories", "public-page"), &token),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
}

#[tokio::test]
async fn a_scoped_key_is_refused_for_another_tenant() {
    let env = env("scoped-other-tenant").await;
    let (_, token) = mint(&env, &["site_reader"]);
    let mut req = get(&node_uri(REPO, "stories", "public-page"), &token);
    req.headers_mut()
        .insert("x-tenant-id", "some-other-tenant".parse().unwrap());
    let (status, text) = send(&env.app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
}

#[tokio::test]
async fn a_scoped_key_is_refused_on_management_and_account_routes() {
    let env = env("scoped-management").await;
    let (_, token) = mint(&env, &["site_reader"]);
    for uri in [
        // A SignedIn read on the admin gate: a scoped key is refused before it.
        format!("/api/management/repositories/{TENANT}/{REPO}/system-updates"),
        "/api/management/system-definitions".to_string(),
        // Minting further keys (require_auth: JWTs only).
        "/api/raisindb/me/api-keys".to_string(),
        "/api/repositories".to_string(),
    ] {
        let (status, text) = send(&env.app, get(&uri, &token)).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
            "{uri}: {status} {text}"
        );
    }
}

#[tokio::test]
async fn an_unscoped_admin_key_is_unchanged() {
    let env = env("unscoped").await;
    for (ws, name) in [("stories", "public-page"), ("secrets", "private-note")] {
        let (status, text) = send(&env.app, get(&node_uri(REPO, ws, name), &env.admin_key)).await;
        assert_eq!(status, StatusCode::OK, "system context reads {ws}: {text}");
    }
    // And in any repository.
    let (status, _) = send(
        &env.app,
        get(&node_uri(OTHER, "secrets", "private-note"), &env.admin_key),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_revoked_scoped_key_reads_nothing() {
    let env = env("scoped-revoke").await;
    let (key_id, token) = mint(&env, &["site_reader"]);
    let uri = node_uri(REPO, "stories", "public-page");
    assert_eq!(send(&env.app, get(&uri, &token)).await.0, StatusCode::OK);

    assert!(env.auth.revoke_scoped_api_key(TENANT, &key_id).unwrap());
    assert_ne!(send(&env.app, get(&uri, &token)).await.0, StatusCode::OK);
    // Revoking again, or an unknown key, is not an error.
    assert!(!env
        .auth
        .revoke_scoped_api_key(TENANT, "no-such-key")
        .unwrap());
    let listed = env.auth.list_scoped_api_keys(TENANT).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].is_active);
}

/// The creation-time checks the operator route makes, through the same
/// `PermissionService` calls: unknown roles are unknown, and a role that can
/// write the ACL workspace unconditionally counts as administration — and a
/// key that somehow holds one resolves to no permissions at all.
#[tokio::test]
async fn role_checks_and_the_shared_resolver() {
    let env = env("scoped-roles").await;
    let service = raisin_core::PermissionService::new(env.storage.clone());
    assert!(service
        .role_exists(TENANT, REPO, "main", "site_reader")
        .await
        .unwrap());
    assert!(!service
        .role_exists(TENANT, REPO, "main", "no_such_role")
        .await
        .unwrap());
    assert!(!service
        .role_exists(TENANT, OTHER, "main", "site_reader")
        .await
        .unwrap());

    let reader = service
        .resolve_for_roles(TENANT, REPO, "main", "apikey:x", &["site_reader".into()])
        .await
        .unwrap();
    assert_eq!(raisin_core::admin_grant_in(&reader), None);
    let writer = service
        .resolve_for_roles(TENANT, REPO, "main", "apikey:x", &["acl_writer".into()])
        .await
        .unwrap();
    assert!(raisin_core::admin_grant_in(&writer).is_some());

    let (key, _) = env
        .auth
        .create_scoped_api_key(
            TENANT,
            "smuggled",
            ApiKeyScope {
                repo: REPO.into(),
                roles: vec!["acl_writer".into()],
            },
        )
        .unwrap();
    let ctx = service
        .resolve_scoped_key(&key, "main")
        .await
        .unwrap()
        .expect("scoped");
    assert!(!ctx.is_system);
    assert!(ctx.permissions().is_some_and(|p| p.permissions.is_empty()));
    assert_eq!(
        ctx.user_id.as_deref(),
        Some(&*format!("apikey:{}", key.key_id))
    );

    // An unscoped key has no scoped context: its owner's authority applies.
    let plain = raisin_models::api_key::ApiKey::new(
        "k".into(),
        "u".into(),
        TENANT.into(),
        "n".into(),
        "h".into(),
        "p".into(),
    );
    assert!(service
        .resolve_scoped_key(&plain, "main")
        .await
        .unwrap()
        .is_none());
}
