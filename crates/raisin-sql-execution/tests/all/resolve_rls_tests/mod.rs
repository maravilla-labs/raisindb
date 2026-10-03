//! `RESOLVE()` is a read, and every read is checked.
//!
//! RESOLVE used to build its resolver from tenant, repo and branch alone, so a
//! caller who could read ONE page could inline any node that page — or a JSON
//! literal the caller typed — referenced, in any workspace. These pin the fix:
//! every target goes through row-level security, and a denied target comes back
//! exactly as a missing one does, so RESOLVE is no existence oracle either.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

mod graph;
mod locale;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
/// Readable by the restricted reader.
const PAGES: &str = "pages";
/// NOT readable by the restricted reader.
const SECRET: &str = "secret";
/// A second workspace holding the same paths as `assets`.
const MEDIA: &str = "media";
const ASSETS: &str = "assets";

type Store = raisin_rocksdb::RocksDBStorage;

async fn setup() -> (Arc<Store>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = Store::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
    for ws in [PAGES, SECRET, MEDIA, ASSETS] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .expect("workspace");
    }
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(json!({ "name": "test:Doc" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (storage, temp_dir)
}

fn engine(storage: &Arc<Store>, auth: AuthContext) -> QueryEngine<Store> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    for ws in [PAGES, SECRET, MEDIA, ASSETS] {
        catalog.register_workspace(ws.to_string());
    }
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(auth)
}

/// Read on `pages` and `assets`, nothing on `secret`.
fn page_reader() -> AuthContext {
    AuthContext::for_user("reader").with_permissions(ResolvedPermissions {
        user_id: "reader".into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![
            Permission::new("/**", vec![Operation::Read]).with_workspace(PAGES),
            Permission::new("/**", vec![Operation::Read]).with_workspace(ASSETS),
        ],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

async fn rows(engine: &QueryEngine<Store>, sql: &str) -> Vec<Value> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        out.push(serde_json::to_value(&row.columns).expect("row as json"));
    }
    out
}

/// The error a statement fails with, whether at planning or while streaming.
async fn error_of(engine: &QueryEngine<Store>, sql: &str) -> String {
    let mut stream = match engine.execute(sql).await {
        Ok(stream) => stream,
        Err(e) => return e.to_string(),
    };
    while let Some(row) = stream.next().await {
        if let Err(e) = row {
            return e.to_string();
        }
    }
    panic!("expected [{sql}] to fail");
}

async fn insert(engine: &QueryEngine<Store>, ws: &str, id: &str, path: &str, props: Value) {
    let props = props.to_string().replace('\'', "''");
    rows(
        engine,
        &format!(
            "INSERT INTO '{ws}' (id, path, node_type, properties) \
             VALUES ('{id}', '{path}', 'test:Doc', '{props}'::jsonb)"
        ),
    )
    .await;
}

fn reference(id: &str, ws: &str) -> Value {
    json!({ "raisin:ref": id, "raisin:workspace": ws })
}

/// One page in `pages` referencing a secret, a missing node, and an asset.
async fn seed(storage: &Arc<Store>) {
    let sys = engine(storage, AuthContext::system());
    insert(&sys, SECRET, "s1", "/s1", json!({ "pii": "salary" })).await;
    insert(&sys, ASSETS, "a1", "/a1", json!({ "alt": "A plane" })).await;
    insert(
        &sys,
        PAGES,
        "home",
        "/home",
        json!({
            "denied": reference("s1", SECRET),
            "missing": reference("nope", SECRET),
            "allowed": reference("a1", ASSETS),
        }),
    )
    .await;
}

async fn resolved(engine: &QueryEngine<Store>, projection: &str) -> Value {
    let found = rows(
        engine,
        &format!("SELECT {projection} AS r FROM '{PAGES}' WHERE path = '/home'"),
    )
    .await;
    found[0]["r"].clone()
}

#[tokio::test]
async fn resolve_respects_rls_on_referenced_nodes() {
    let (storage, _dir) = setup().await;
    seed(&storage).await;

    // The target exists and a system caller inlines it...
    let sys = resolved(
        &engine(&storage, AuthContext::system()),
        "RESOLVE(properties)",
    )
    .await;
    assert_eq!(sys["denied"]["pii"], "salary");

    // ...but the reader gets the reference back exactly as stored — the same
    // treatment a missing target gets, so the two cannot be told apart.
    let reader = engine(&storage, page_reader());
    let r = resolved(&reader, "RESOLVE(properties)").await;
    let stored = resolved(&reader, "properties").await;
    assert!(r["denied"].get("pii").is_none(), "secret leaked: {r}");
    assert_eq!(r["denied"], stored["denied"]);
    assert_eq!(r["missing"], stored["missing"]);
    assert_eq!(r["denied"]["raisin:ref"], "s1");
    assert_eq!(r["missing"]["raisin:ref"], "nope");
    // What the reader may read still resolves.
    assert_eq!(r["allowed"]["alt"], "A plane");
}

#[tokio::test]
async fn resolve_literal_jsonb_ref_cannot_read_unauthorized_node() {
    let (storage, _dir) = setup().await;
    seed(&storage).await;

    // A literal the caller typed names the secret directly.
    let literal = r#"RESOLVE('{"raisin:ref":"s1","raisin:workspace":"secret"}'::jsonb)"#;
    let sys = resolved(&engine(&storage, AuthContext::system()), literal).await;
    assert_eq!(sys["pii"], "salary");

    let r = resolved(&engine(&storage, page_reader()), literal).await;
    assert_eq!(
        r,
        json!({ "raisin:ref": "s1", "raisin:workspace": "secret" })
    );

    // Same at depth > 1 and with a fields list.
    let deep =
        r#"RESOLVE('{"x":{"raisin:ref":"s1","raisin:workspace":"secret"}}'::jsonb, 3, 'pii')"#;
    let r = resolved(&engine(&storage, page_reader()), deep).await;
    assert!(r["x"].get("pii").is_none(), "secret leaked: {r}");
}

/// `/logo` in `assets` and `/logo` in `media` are two nodes. The old memo was
/// keyed by the locator alone and handed the second the first one's node.
#[tokio::test]
async fn resolve_same_path_ref_in_two_workspaces_does_not_collide() {
    let (storage, _dir) = setup().await;
    seed(&storage).await;
    let sys = engine(&storage, AuthContext::system());
    insert(
        &sys,
        ASSETS,
        "logo-a",
        "/logo",
        json!({ "v": "from assets" }),
    )
    .await;
    insert(&sys, MEDIA, "logo-m", "/logo", json!({ "v": "from media" })).await;

    let literal = r#"RESOLVE('{"a":{"raisin:ref":"/logo","raisin:workspace":"assets"},"m":{"raisin:ref":"/logo","raisin:workspace":"media"}}'::jsonb)"#;
    let r = resolved(&sys, literal).await;
    assert_eq!(r["a"]["v"], "from assets");
    assert_eq!(r["m"]["v"], "from media");
    assert_eq!(r["a"]["id"], "logo-a");
    assert_eq!(r["m"]["id"], "logo-m");
}

/// A fan-out of ten references per level, five levels deep, is 111,110 inlined
/// nodes from one row — over the 50,000 per-statement bound. The statement
/// fails, naming RESOLVE, instead of returning a truncated document.
#[tokio::test]
async fn resolve_budget_errors_loudly() {
    let (storage, _dir) = setup().await;
    let sys = engine(&storage, AuthContext::system());
    insert(&sys, ASSETS, "n5", "/n5", json!({ "leaf": true })).await;
    for level in (0..5).rev() {
        let child = format!("n{}", level + 1);
        let fan: Vec<Value> = (0..10).map(|_| reference(&child, ASSETS)).collect();
        insert(
            &sys,
            ASSETS,
            &format!("n{level}"),
            &format!("/n{level}"),
            json!({ "children": fan }),
        )
        .await;
    }

    let err = error_of(
        &sys,
        &format!("SELECT RESOLVE(properties, 6) AS r FROM '{ASSETS}' WHERE path = '/n0'"),
    )
    .await;
    assert!(err.contains("RESOLVE"), "{err}");
    assert!(err.contains("budget"), "{err}");

    // Within budget the same graph resolves.
    let found = rows(
        &sys,
        &format!("SELECT RESOLVE(properties, 2) AS r FROM '{ASSETS}' WHERE path = '/n0'"),
    )
    .await;
    let r = &found[0]["r"];
    assert_eq!(r["children"][9]["id"], "n1");
    assert_eq!(r["children"][9]["children"][9]["id"], "n2");
    assert_eq!(
        r["children"][9]["children"][9]["children"][0]["raisin:ref"],
        "n3"
    );
}
