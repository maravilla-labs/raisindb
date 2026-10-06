//! Translation writes over HTTP (`raisin:cmd/translate|hide-in-locale|…`),
//! over WebSocket (`translation_update|translation_hide|…`) and over SQL
//! (`UPDATE … FOR LOCALE`) share ONE policy
//! (`NodeService::authorize_translation_write` in raisin-core): resolution,
//! readability, the `Translate` permission (graph `RELATES` included), id keying
//! and the actor. SQL keeps only its own transactional write. Proved here
//! against one RocksDB, over the real HTTP router, the real WS dispatcher and
//! the real SQL engine:
//!
//! - a WS hide is keyed by the node ID, so `WHERE path = $1 AND locale = 'fr'`
//!   stops returning the node (the old WS handler keyed it by PATH, so the
//!   overlay was invisible to every reader);
//! - a caller who may read the node but not update it is refused on BOTH wires
//!   and nothing is written;
//! - the recorded actor is the authenticated caller — not `"system"` (old WS),
//!   and not an `actor` named in the request body (old HTTP);
//! - the same input produces the same overlay on both wires.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures::StreamExt;
use parking_lot::RwLock;
use serde_json::{json, Value};

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_models::translations::{LocaleCode, LocaleOverlay};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, NodeRepository, Storage, StorageScope, TranslationRepository,
};
use raisin_transport_ws::{
    handlers::route_request, ConnectionState, RequestEnvelope, RequestType, ResponseEnvelope,
    ResponseStatus, WsConfig, WsState,
};

use crate::support::{admin, create_at, send, Fixture, TENANT};

const REPO: &str = "trcmd";
const BRANCH: &str = "main";
const WS: &str = "content";
const NT: &str = "t";

type Ws = WsState<RocksDBStorage, raisin_binary::FilesystemBinaryStorage>;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct World {
    fx: Fixture,
    ws: Arc<Ws>,
}

impl World {
    fn storage(&self) -> &Arc<RocksDBStorage> {
        &self.fx.storage
    }
}

async fn world(label: &str) -> World {
    let fx = Fixture::new(label, REPO, BRANCH, &[WS], &[NT]).await;
    let base = format!("/api/repository/{REPO}/{BRANCH}/head/{WS}");
    for path in [
        "/locked",
        "/locked/page",
        "/writable",
        "/writable/page",
        "/hide-me",
        "/via-http",
        "/via-ws",
        "/via-sql",
        "/owner",
        "/owned",
        "/unowned",
    ] {
        let name = path.rsplit('/').next().unwrap();
        let id = format!("id-{}", path.trim_start_matches('/').replace('/', "-"));
        let (status, text) = create_at(&fx.app, &base, &id, name, path, NT).await;
        assert!(status.is_success(), "create {path}: {status} {text}");
    }
    anonymous_role(&fx).await;

    let storage = fx.storage.clone();
    let ws = Arc::new(WsState::new(
        storage.clone(),
        Arc::new(raisin_core::RaisinConnection::with_storage(storage.clone())),
        Arc::new(raisin_core::WorkspaceService::new(storage.clone())),
        Arc::new(raisin_binary::FilesystemBinaryStorage::new(
            "./.data/uploads",
            Some("/files".into()),
        )),
        WsConfig::default(),
        None,
        None,
        None,
        None,
        None,
        None,
        Arc::new(storage.audit_repository()),
    ));
    World { fx, ws }
}

fn acl_node(
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
        properties: props,
        parent: Some(parent.to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(workspace.to_string()),
        ..Default::default()
    }
}

fn grant(path: &str, ops: &[&str]) -> PropertyValue {
    PropertyValue::Object(HashMap::from([
        ("workspace".to_string(), PropertyValue::String(WS.into())),
        ("path".to_string(), PropertyValue::String(path.into())),
        (
            "operations".to_string(),
            PropertyValue::Array(
                ops.iter()
                    .map(|o| PropertyValue::String((*o).into()))
                    .collect(),
            ),
        ),
    ]))
}

/// Anonymous access on, and the anonymous caller holding: read everywhere in
/// `content`, update only under `/writable`. Same shape `http_asset_public`
/// uses, so the HTTP request runs as a real, RLS-scoped, non-system principal.
async fn anonymous_role(fx: &Fixture) {
    for ws in ["raisin:system", "raisin:access_control"] {
        crate::support::create_workspace(&fx.app, REPO, ws).await;
    }
    let nodes = fx.storage.nodes_impl();
    let add = |ws: &'static str, n| {
        let nodes = nodes.clone();
        async move { nodes.add(TENANT, REPO, BRANCH, ws, n).await.unwrap() }
    };
    let sys = "raisin:system";
    let acl = "raisin:access_control";
    add(
        sys,
        acl_node(
            "config",
            "/config",
            "raisin:Folder",
            "/",
            sys,
            HashMap::new(),
        ),
    )
    .await;
    add(
        sys,
        acl_node(
            "repos",
            "/config/repos",
            "raisin:Folder",
            "config",
            sys,
            HashMap::new(),
        ),
    )
    .await;
    add(
        sys,
        acl_node(
            REPO,
            &format!("/config/repos/{REPO}"),
            "raisin:RepoAuthConfig",
            "repos",
            sys,
            HashMap::from([(
                "anonymous_enabled".to_string(),
                PropertyValue::Boolean(true),
            )]),
        ),
    )
    .await;
    for folder in ["users", "roles"] {
        add(
            acl,
            acl_node(
                folder,
                &format!("/{folder}"),
                "raisin:AclFolder",
                "/",
                acl,
                HashMap::new(),
            ),
        )
        .await;
    }
    let role = HashMap::from([
        (
            "role_id".to_string(),
            PropertyValue::String("anonymous".into()),
        ),
        (
            "name".to_string(),
            PropertyValue::String("anonymous".into()),
        ),
        ("inherits".to_string(), PropertyValue::Array(vec![])),
        (
            "permissions".to_string(),
            PropertyValue::Array(vec![
                grant("/**", &["read"]),
                grant("/writable/**", &["read", "translate"]),
            ]),
        ),
    ]);
    add(
        acl,
        acl_node(
            "anonymous-role",
            "/roles/anonymous",
            "raisin:Role",
            "roles",
            acl,
            role,
        ),
    )
    .await;
    let user = HashMap::from([
        (
            "user_id".to_string(),
            PropertyValue::String("anonymous".into()),
        ),
        (
            "roles".to_string(),
            PropertyValue::Array(vec![PropertyValue::String("anonymous".into())]),
        ),
    ]);
    add(
        acl,
        acl_node(
            "anonymous-user",
            "/users/anonymous",
            "raisin:User",
            "users",
            acl,
            user,
        ),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Wires
// ---------------------------------------------------------------------------

/// `POST <node>/raisin:cmd/<command>`; `as_admin = false` sends no credential,
/// i.e. as the anonymous principal configured above.
async fn http_cmd(
    w: &World,
    node_path: &str,
    command: &str,
    body: Value,
    as_admin: bool,
) -> (StatusCode, String) {
    let uri = format!("/api/repository/{REPO}/{BRANCH}/head/{WS}{node_path}/raisin:cmd/{command}");
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if as_admin {
        req = admin(req);
    }
    send(
        &w.fx.raw,
        req.body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
}

fn user(user_id: &str, permissions: Vec<Permission>) -> AuthContext {
    AuthContext::for_user(user_id).with_permissions(ResolvedPermissions {
        user_id: user_id.to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions,
        is_system_admin: false,
        resolved_at: None,
    })
}

fn writer(user_id: &str) -> AuthContext {
    user(
        user_id,
        vec![Permission::new(
            "**",
            vec![Operation::Read, Operation::Translate],
        )],
    )
}

fn editor_without_translate(user_id: &str) -> AuthContext {
    user(
        user_id,
        vec![Permission::new(
            "**",
            vec![Operation::Read, Operation::Update],
        )],
    )
}

fn reader(user_id: &str) -> AuthContext {
    user(user_id, vec![Permission::new("**", vec![Operation::Read])])
}

/// Dispatch one request through the WS router, as a connection authenticated as `auth`.
async fn ws_call(
    w: &World,
    auth: AuthContext,
    request_type: RequestType,
    payload: Value,
) -> Result<ResponseEnvelope, raisin_transport_ws::WsError> {
    let mut conn = ConnectionState::new(TENANT.to_string(), Some(REPO.to_string()), 8, 100);
    conn.set_user_id(auth.user_id.clone().unwrap_or_default());
    conn.set_auth_context(auth);
    let conn = Arc::new(RwLock::new(conn));
    let request = RequestEnvelope {
        request_id: uuid::Uuid::new_v4().to_string(),
        request_type,
        context: raisin_transport_ws::protocol::RequestContext {
            tenant_id: TENANT.to_string(),
            repository: Some(REPO.to_string()),
            branch: Some(BRANCH.to_string()),
            workspace: Some(WS.to_string()),
            revision: None,
        },
        payload,
    };
    route_request(&w.ws, &conn, request)
        .await
        .map(|r| r.expect("translation requests answer inline"))
}

// ---------------------------------------------------------------------------
// Readers
// ---------------------------------------------------------------------------

async fn node_id(w: &World, path: &str) -> String {
    w.storage()
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{path} exists"))
        .id
}

async fn overlay(w: &World, node_id: &str, locale: &str) -> Option<LocaleOverlay> {
    let head = w
        .storage()
        .branches()
        .get_head(TENANT, REPO, BRANCH)
        .await
        .unwrap();
    w.storage()
        .translations()
        .get_translation(
            TENANT,
            REPO,
            BRANCH,
            WS,
            node_id,
            &LocaleCode::parse(locale).unwrap(),
            &head,
        )
        .await
        .unwrap()
}

async fn actor(w: &World, node_id: &str, locale: &str) -> String {
    w.storage()
        .translations()
        .get_translation_meta(
            TENANT,
            REPO,
            BRANCH,
            WS,
            node_id,
            &LocaleCode::parse(locale).unwrap(),
        )
        .await
        .unwrap()
        .expect("translation meta recorded")
        .actor
}

fn sql_engine(w: &World, auth: AuthContext) -> raisin_sql_execution::QueryEngine<RocksDBStorage> {
    let mut catalog = raisin_sql_execution::StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    raisin_sql_execution::QueryEngine::new(w.storage().clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_repository_config(raisin_context::RepositoryConfig {
            default_language: "en".to_string(),
            supported_languages: vec!["en".into(), "fr".into(), "de".into()],
            default_branch: BRANCH.to_string(),
            ..raisin_context::RepositoryConfig::default()
        })
        .with_auth(auth)
}

/// Run one SQL statement as `auth` to completion; the first error, if any.
async fn sql(w: &World, auth: AuthContext, statement: &str) -> Result<(), raisin_error::Error> {
    let mut stream = sql_engine(w, auth).execute(statement).await?;
    while let Some(row) = stream.next().await {
        row?;
    }
    Ok(())
}

/// `UPDATE content FOR LOCALE 'fr' SET title = 'x' WHERE path = '<path>'`.
async fn sql_translate(
    w: &World,
    auth: AuthContext,
    path: &str,
) -> Result<(), raisin_error::Error> {
    sql(
        w,
        auth,
        &format!("UPDATE {WS} FOR LOCALE 'fr' SET title = 'x' WHERE path = '{path}'"),
    )
    .await
}

/// `SELECT id FROM content WHERE path = $1 AND locale = 'fr'`, as the system.
async fn select_in_fr(w: &World, path: &str) -> Vec<String> {
    let engine = sql_engine(w, AuthContext::system());
    let format = |v: &Value| match v {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        other => other.to_string(),
    };
    let mut stream = engine
        .execute_with_params(
            &format!("SELECT id FROM '{WS}' WHERE path = $1 AND locale = 'fr'"),
            &[json!(path)],
            &format,
        )
        .await
        .expect("query runs");
    let mut ids = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("row");
        for (col, value) in row.columns {
            if col == "id" || col.ends_with(".id") {
                if let PropertyValue::String(s) = value {
                    ids.push(s);
                }
            }
        }
    }
    ids
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// (a) A WS hide lands on the node's ID, so a locale read omits the node.
#[tokio::test]
async fn ws_hide_is_keyed_by_node_id_and_hides_the_node_in_sql() {
    let w = world("ws-hide").await;
    let id = node_id(&w, "/hide-me").await;
    assert_eq!(
        select_in_fr(&w, "/hide-me").await,
        vec![id.clone()],
        "visible before hide"
    );

    let resp = ws_call(
        &w,
        writer("alice"),
        RequestType::TranslationHide,
        json!({ "node_path": "/hide-me", "locale": "fr" }),
    )
    .await
    .expect("hide succeeds");
    assert!(matches!(resp.status, ResponseStatus::Success));
    assert_eq!(resp.result.unwrap()["node_id"], json!(id));

    assert!(matches!(
        overlay(&w, &id, "fr").await,
        Some(LocaleOverlay::Hidden)
    ));
    assert!(
        overlay(&w, "/hide-me", "fr").await.is_none(),
        "nothing may be keyed by the path"
    );
    assert!(
        select_in_fr(&w, "/hide-me").await.is_empty(),
        "hidden in fr"
    );

    // Unhide restores it.
    ws_call(
        &w,
        writer("alice"),
        RequestType::TranslationUnhide,
        json!({ "node_path": "/hide-me", "locale": "fr" }),
    )
    .await
    .expect("unhide succeeds");
    assert_eq!(select_in_fr(&w, "/hide-me").await, vec![id]);
}

/// (b) Read-but-not-translate is refused on both wires, and nothing is written.
#[tokio::test]
async fn a_caller_without_translate_permission_is_refused_on_http_and_ws() {
    let w = world("refused").await;
    let id = node_id(&w, "/locked/page").await;

    // HTTP, as the anonymous principal: may read /locked/page, may not translate it.
    for (command, body) in [
        ("hide-in-locale", json!({ "locale": "fr" })),
        (
            "translate",
            json!({ "locale": "fr", "translations": { "/title": "x" } }),
        ),
        ("delete-translation", json!({ "locale": "fr" })),
        ("unhide-in-locale", json!({ "locale": "fr" })),
    ] {
        let (status, text) = http_cmd(&w, "/locked/page", command, body, false).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "HTTP {command}: {text}");
    }

    // WS, as a user with read only.
    for (request_type, payload) in [
        (
            RequestType::TranslationHide,
            json!({ "node_path": "/locked/page", "locale": "fr" }),
        ),
        (
            RequestType::TranslationUpdate,
            json!({ "node_path": "/locked/page", "locale": "fr", "properties": { "title": "x" } }),
        ),
        (
            RequestType::TranslationDelete,
            json!({ "node_path": "/locked/page", "locale": "fr" }),
        ),
        (
            RequestType::TranslationUnhide,
            json!({ "node_path": "/locked/page", "locale": "fr" }),
        ),
        // By id too: the id form must not be a way around the check.
        (
            RequestType::TranslationHide,
            json!({ "node_path": id, "locale": "fr" }),
        ),
    ] {
        let err = ws_call(&w, reader("bob"), request_type, payload)
            .await
            .expect_err("WS write must be refused");
        assert!(
            matches!(err, raisin_transport_ws::WsError::PermissionDenied),
            "WS: {err:?}"
        );
    }

    // A caller who cannot even READ the node gets not-found, not a refusal.
    let err = ws_call(
        &w,
        user("eve", vec![]),
        RequestType::TranslationHide,
        json!({ "node_path": "/locked/page", "locale": "fr" }),
    )
    .await
    .expect_err("unreadable");
    assert!(
        !matches!(err, raisin_transport_ws::WsError::PermissionDenied),
        "{err:?}"
    );

    assert!(overlay(&w, &id, "fr").await.is_none(), "nothing written");
    assert_eq!(select_in_fr(&w, "/locked/page").await, vec![id]);

    // Reading the locale list needs only read.
    let resp = ws_call(
        &w,
        reader("bob"),
        RequestType::TranslationList,
        json!({ "node_path": "/locked/page" }),
    )
    .await
    .expect("list with read");
    assert!(matches!(resp.status, ResponseStatus::Success));
}

/// (c) The recorded actor is the authenticated caller on both wires.
#[tokio::test]
async fn the_recorded_actor_is_the_authenticated_caller() {
    let w = world("actor").await;

    // WS: the connection's user, not "system".
    let ws_id = node_id(&w, "/via-ws").await;
    ws_call(
        &w,
        writer("alice"),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/via-ws", "locale": "fr", "properties": { "title": "Bonjour" } }),
    )
    .await
    .expect("WS translate");
    assert_eq!(actor(&w, &ws_id, "fr").await, "alice");

    // HTTP: the anonymous principal may update under /writable; the body's
    // `actor` is not a way to sign someone else's name.
    let http_id = node_id(&w, "/writable/page").await;
    let (status, text) = http_cmd(
        &w,
        "/writable/page",
        "translate",
        json!({ "locale": "fr", "translations": { "/title": "Bonjour" }, "actor": "mallory" }),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let recorded = actor(&w, &http_id, "fr").await;
    assert_ne!(recorded, "mallory");
    // The anonymous principal's user id is its `raisin:User` node id.
    assert_eq!(recorded, "anonymous-user");
}

/// (d) The same input gives the same overlay over either wire.
#[tokio::test]
async fn http_and_ws_produce_the_same_overlay() {
    let w = world("parity").await;
    let fields = json!({
        "title": "Bonjour",
        "count": 3,
        "ratio": 1.5,
        "published": true,
        "tags": ["a", "b"],
        "seo": { "description": "Une page" },
    });

    let http_fields: serde_json::Map<String, Value> = fields
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (format!("/{k}"), v.clone()))
        .collect();
    let (status, text) = http_cmd(
        &w,
        "/via-http",
        "translate",
        json!({ "locale": "fr", "translations": http_fields }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    ws_call(
        &w,
        writer("alice"),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/via-ws", "locale": "fr", "properties": fields }),
    )
    .await
    .expect("WS translate");

    let http_overlay = overlay(&w, &node_id(&w, "/via-http").await, "fr").await;
    let ws_overlay = overlay(&w, &node_id(&w, "/via-ws").await, "fr").await;
    let (Some(LocaleOverlay::Properties { data: h }), Some(LocaleOverlay::Properties { data: s })) =
        (http_overlay, ws_overlay)
    else {
        panic!("both wires must write a properties overlay");
    };
    assert_eq!(h.len(), 6);
    assert_eq!(h, s);

    // Hide parity too: both answer with the node id, both write `Hidden` on it.
    let (status, text) = http_cmd(
        &w,
        "/via-http",
        "hide-in-locale",
        json!({ "locale": "de" }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let http_body: Value = serde_json::from_str(&text).unwrap();
    let resp = ws_call(
        &w,
        writer("alice"),
        RequestType::TranslationHide,
        json!({ "node_path": "/via-ws", "locale": "de" }),
    )
    .await
    .unwrap();
    let ws_body = resp.result.unwrap();
    for (body, path) in [(&http_body, "/via-http"), (&ws_body, "/via-ws")] {
        let id = node_id(&w, path).await;
        assert_eq!(body["node_id"], json!(id));
        assert_eq!(body["locale"], json!("de"));
        assert!(matches!(
            overlay(&w, &id, "de").await,
            Some(LocaleOverlay::Hidden)
        ));
    }
}

/// Translating is its own grant: `Translate` without `Update` may translate and
/// hide; `Update` without `Translate` may not. Same rule as SQL `FOR LOCALE`.
#[tokio::test]
async fn translate_is_its_own_permission_independent_of_update() {
    let w = world("translate-grant").await;
    let id = node_id(&w, "/locked/page").await;

    let err = ws_call(
        &w,
        editor_without_translate("ed"),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/locked/page", "locale": "fr", "properties": { "title": "x" } }),
    )
    .await
    .expect_err("update without translate must be refused");
    assert!(
        matches!(err, raisin_transport_ws::WsError::PermissionDenied),
        "{err:?}"
    );
    assert!(overlay(&w, &id, "fr").await.is_none(), "nothing written");

    // read + translate, no update: a translator role
    ws_call(
        &w,
        writer("tina"),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/locked/page", "locale": "fr", "properties": { "title": "Titre" } }),
    )
    .await
    .expect("translate without update is allowed");
    assert!(overlay(&w, &id, "fr").await.is_some(), "overlay written");
    ws_call(
        &w,
        writer("tina"),
        RequestType::TranslationHide,
        json!({ "node_path": "/locked/page", "locale": "fr" }),
    )
    .await
    .expect("hide with translate");
    assert!(select_in_fr(&w, "/locked/page").await.is_empty());
}

// ---------------------------------------------------------------------------
// SQL `UPDATE … FOR LOCALE`: the same policy as HTTP and WS
// ---------------------------------------------------------------------------

/// The matrix over SQL, with WS alongside where the verdicts must agree:
/// read-only and update-without-translate refused, unreadable (including
/// translate-without-read) not found, translate-without-update allowed, the
/// overlay keyed by id, the actor the caller.
#[tokio::test]
async fn sql_for_locale_applies_the_same_policy_as_http_and_ws() {
    let w = world("sql-policy").await;
    let id = node_id(&w, "/locked/page").await;
    let by_id = format!("UPDATE {WS} FOR LOCALE 'fr' SET title = 'x' WHERE id = '{id}'");

    for auth in [reader("bob"), editor_without_translate("ed")] {
        let err = sql_translate(&w, auth.clone(), "/locked/page")
            .await
            .expect_err("SQL by path must be refused");
        assert!(
            matches!(err, raisin_error::Error::PermissionDenied(_)),
            "{err:?}"
        );
        let err = sql(&w, auth.clone(), &by_id)
            .await
            .expect_err("SQL by id must be refused");
        assert!(
            matches!(err, raisin_error::Error::PermissionDenied(_)),
            "{err:?}"
        );
        let err = ws_call(
            &w,
            auth,
            RequestType::TranslationUpdate,
            json!({ "node_path": "/locked/page", "locale": "fr", "properties": { "title": "x" } }),
        )
        .await
        .expect_err("WS must be refused");
        assert!(
            matches!(err, raisin_transport_ws::WsError::PermissionDenied),
            "{err:?}"
        );
    }

    // Unreadable is not found — never a refusal that confirms the node exists.
    // Holding `Translate` without `Read` does not change that.
    let translate_only = user(
        "tom",
        vec![Permission::new("**", vec![Operation::Translate])],
    );
    for auth in [user("eve", vec![]), translate_only] {
        for statement in [
            format!("UPDATE {WS} FOR LOCALE 'fr' SET title = 'x' WHERE path = '/locked/page'"),
            by_id.clone(),
        ] {
            let err = sql(&w, auth.clone(), &statement)
                .await
                .expect_err("unreadable");
            assert!(
                matches!(err, raisin_error::Error::NotFound(_)),
                "{statement}: {err:?}"
            );
        }
        let err = ws_call(
            &w,
            auth,
            RequestType::TranslationUpdate,
            json!({ "node_path": "/locked/page", "locale": "fr", "properties": { "title": "x" } }),
        )
        .await
        .expect_err("unreadable over WS");
        assert!(
            !matches!(err, raisin_transport_ws::WsError::PermissionDenied),
            "{err:?}"
        );
    }
    assert!(overlay(&w, &id, "fr").await.is_none(), "nothing written");

    // Translate without Update: allowed, keyed by the id, attributed to the caller.
    sql_translate(&w, writer("tina"), "/locked/page")
        .await
        .expect("translate without update is allowed over SQL");
    let Some(LocaleOverlay::Properties { data }) = overlay(&w, &id, "fr").await else {
        panic!("SQL must write a properties overlay on the node id");
    };
    assert_eq!(data.len(), 1);
    assert!(
        overlay(&w, "/locked/page", "fr").await.is_none(),
        "nothing may be keyed by the path"
    );
    assert_eq!(actor(&w, &id, "fr").await, "tina");
}

/// A `Translate` grant conditioned on the relation graph is evaluated with a
/// graph resolver on every surface: the related node may be translated, the
/// unrelated one may not.
#[tokio::test]
async fn a_relates_conditioned_translate_grant_is_honoured_on_sql_and_ws() {
    let w = world("sql-graph").await;
    let owner = node_id(&w, "/owner").await;
    let owned = node_id(&w, "/owned").await;
    let unowned = node_id(&w, "/unowned").await;
    sql(
        &w,
        AuthContext::system(),
        &format!(
            "RELATE FROM id='{owned}' IN WORKSPACE '{WS}' \
             TO id='{owner}' IN WORKSPACE '{WS}' TYPE 'OWNED_BY'"
        ),
    )
    .await
    .expect("relate");

    let translator = || {
        user(
            "owner",
            vec![
                Permission::new("**", vec![Operation::Read]),
                Permission::new("**", vec![Operation::Translate])
                    .with_condition("node.id RELATES auth.local_user_id VIA 'OWNED_BY'"),
            ],
        )
        .with_local_user_id(owner.clone())
    };
    assert!(translator().uses_graph_rls());

    sql_translate(&w, translator(), "/owned")
        .await
        .expect("SQL: related node is translatable");
    assert!(overlay(&w, &owned, "fr").await.is_some());
    ws_call(
        &w,
        translator(),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/owned", "locale": "de", "properties": { "title": "Hallo" } }),
    )
    .await
    .expect("WS: related node is translatable");
    assert!(overlay(&w, &owned, "de").await.is_some());

    let err = sql_translate(&w, translator(), "/unowned")
        .await
        .expect_err("SQL: unrelated node refused");
    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "{err:?}"
    );
    let err = ws_call(
        &w,
        translator(),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/unowned", "locale": "fr", "properties": { "title": "x" } }),
    )
    .await
    .expect_err("WS: unrelated node refused");
    assert!(
        matches!(err, raisin_transport_ws::WsError::PermissionDenied),
        "{err:?}"
    );
    assert!(overlay(&w, &unowned, "fr").await.is_none());
}

/// The same scalar input gives the same overlay over HTTP, WS and SQL, and SQL
/// records the caller as the actor, like the other two.
#[tokio::test]
async fn http_ws_and_sql_produce_the_same_overlay() {
    let w = world("sql-parity").await;
    let fields = json!({ "title": "Bonjour", "views": 3, "ratio": 1.5, "featured": true });

    let http_fields: serde_json::Map<String, Value> = fields
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (format!("/{k}"), v.clone()))
        .collect();
    let (status, text) = http_cmd(
        &w,
        "/via-http",
        "translate",
        json!({ "locale": "fr", "translations": http_fields }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    ws_call(
        &w,
        writer("alice"),
        RequestType::TranslationUpdate,
        json!({ "node_path": "/via-ws", "locale": "fr", "properties": fields }),
    )
    .await
    .expect("WS translate");
    sql(
        &w,
        writer("alice"),
        &format!(
            "UPDATE {WS} FOR LOCALE 'fr' SET title = 'Bonjour', views = 3, \
             ratio = 1.5, featured = true WHERE path = '/via-sql'"
        ),
    )
    .await
    .expect("SQL translate");

    let mut overlays = Vec::new();
    for path in ["/via-http", "/via-ws", "/via-sql"] {
        match overlay(&w, &node_id(&w, path).await, "fr").await {
            Some(LocaleOverlay::Properties { data }) => overlays.push(data),
            other => panic!("{path}: expected a properties overlay, got {other:?}"),
        }
    }
    assert_eq!(overlays[0].len(), 4);
    assert_eq!(overlays[0], overlays[1], "HTTP vs WS");
    assert_eq!(overlays[0], overlays[2], "HTTP vs SQL");

    let sql_id = node_id(&w, "/via-sql").await;
    assert_eq!(actor(&w, &sql_id, "fr").await, "alice");
}
