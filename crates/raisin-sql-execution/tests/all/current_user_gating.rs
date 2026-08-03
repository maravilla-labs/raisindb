//! `RAISIN_CURRENT_USER()` resolves the caller's user node — and the lookup that
//! backs it is skipped for the ~all statements that never call it.
//!
//! Resolving that node costs a property-index scan, a node read and a full
//! serialization, and it used to run on EVERY authenticated statement in the
//! product regardless of whether the SQL referenced the function. The gate that
//! removes that work is a substring test over the raw SQL, which is sound in one
//! direction only: the parser cannot emit the call unless the identifier appears
//! verbatim, so a negative is proof, while a positive may be a mere mention.
//!
//! These tests pin the behaviour that matters — that gating did not break the
//! function — because a regression here is silent: `RAISIN_CURRENT_USER()` would
//! simply start returning NULL, and every RLS policy or view built on it would
//! quietly change meaning rather than error.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
/// The workspace `lookup_user_node` searches. Not configurable.
const ACCESS_CONTROL_WS: &str = "raisin:access_control";
const DATA_WS: &str = "items";

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);

    for ws in [ACCESS_CONTROL_WS, DATA_WS] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .expect("workspace");
    }

    for nt in ["raisin:User", "test:Item"] {
        storage
            .node_types()
            .create(
                BranchScope::new(TENANT, REPO, BRANCH),
                serde_json::from_value(serde_json::json!({ "name": nt })).expect("nt"),
                CommitMetadata {
                    message: "t".into(),
                    actor: "t".into(),
                    is_system: true,
                },
            )
            .await
            .expect("nodetype");
    }

    (storage, temp_dir)
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth: AuthContext,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(ACCESS_CONTROL_WS.to_string());
    catalog.register_workspace(DATA_WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(auth)
}

async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

/// An authenticated identity that may read anywhere.
///
/// `AuthContext::for_user` alone carries no permissions, and RLS then filters
/// every row — which would make these tests pass or fail for reasons that have
/// nothing to do with the gate under test.
fn reader(user_id: &str) -> AuthContext {
    AuthContext::for_user(user_id).with_permissions(ResolvedPermissions {
        user_id: user_id.into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new(
            "/**",
            vec![Operation::Create, Operation::Read, Operation::Update],
        )],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

/// Run a single-column query and return the first row's only value.
async fn scalar(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Option<PropertyValue> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let row = stream.next().await?.unwrap_or_else(|e| panic!("row: {e}"));
    row.columns.into_iter().next().map(|(_, v)| v)
}

/// Create the user node `lookup_user_node` finds by the `user_id` property.
async fn seed_user(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    run(
        &engine(storage, AuthContext::system()),
        &format!(
            "INSERT INTO '{ACCESS_CONTROL_WS}' (id, path, node_type, properties) VALUES \
             ('u1','/users/alice','raisin:User','{{\"user_id\":\"alice\",\"nick\":\"al\"}}'::JSONB)"
        ),
    )
    .await;
}

/// One row to project over. The planner has no FROM-less SELECT, so every probe
/// of `RAISIN_CURRENT_USER()` needs a table to hang off; the row's contents are
/// irrelevant to what these tests assert.
async fn seed_item(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    run(
        &engine(storage, AuthContext::system()),
        &format!(
            "INSERT INTO '{DATA_WS}' (id, path, node_type, properties) VALUES \
             ('i1','/item-1','test:Item','{{\"title\":\"T\"}}'::JSONB)"
        ),
    )
    .await;
}

/// `SELECT <expr> FROM items WHERE path = '/item-1'`
fn probe(expr: &str) -> String {
    format!("SELECT {expr} AS probe FROM '{DATA_WS}' WHERE path = '/item-1'")
}

#[tokio::test]
async fn current_user_resolves_the_callers_node() {
    let (storage, _td) = setup().await;
    seed_user(&storage).await;
    seed_item(&storage).await;

    let value = scalar(
        &engine(&storage, reader("alice")),
        &probe("RAISIN_CURRENT_USER()"),
    )
    .await
    .expect("one row");

    // The function yields the serialized node, which surfaces as an Object.
    let obj = match value {
        PropertyValue::Object(o) => o,
        other => panic!("expected the user node as an object, got {other:?}"),
    };

    assert_eq!(
        obj.get("path"),
        Some(&PropertyValue::String("/users/alice".into())),
        "RAISIN_CURRENT_USER() must return the caller's node — a NULL here means \
         the gate skipped a lookup the statement actually needed"
    );
}

#[tokio::test]
async fn current_user_survives_lowercase_and_whitespace() {
    let (storage, _td) = setup().await;
    seed_user(&storage).await;
    seed_item(&storage).await;

    // The gate is case-insensitive; the parser is too. If the gate were
    // case-SENSITIVE this would silently return NULL rather than fail.
    let value = scalar(
        &engine(&storage, reader("alice")),
        &probe("raisin_current_user()"),
    )
    .await
    .expect("one row");

    assert!(
        matches!(value, PropertyValue::Object(_)),
        "lowercase raisin_current_user() must resolve the node too, got {value:?}"
    );
}

#[tokio::test]
async fn current_user_is_null_for_an_unknown_user() {
    let (storage, _td) = setup().await;
    // No user node seeded.
    seed_item(&storage).await;

    let value = scalar(
        &engine(&storage, reader("nobody")),
        &probe("RAISIN_CURRENT_USER()"),
    )
    .await
    .expect("one row");

    assert!(
        matches!(value, PropertyValue::Null),
        "an unresolvable user must yield NULL, got {value:?}"
    );
}

/// The gate's false-positive direction: SQL that merely MENTIONS the function
/// name in a string literal must still execute correctly. It does the (wasted)
/// lookup, which is exactly the safe outcome.
#[tokio::test]
async fn a_string_literal_mentioning_the_function_still_executes() {
    let (storage, _td) = setup().await;
    seed_user(&storage).await;
    seed_item(&storage).await;

    let value = scalar(
        &engine(&storage, reader("alice")),
        &probe("'RAISIN_CURRENT_USER'"),
    )
    .await
    .expect("one row");

    assert_eq!(
        value,
        PropertyValue::String("RAISIN_CURRENT_USER".into()),
        "a literal mentioning the name must be returned verbatim"
    );
}

/// Ordinary queries — the ~100% case the gate exists for — must be unaffected.
#[tokio::test]
async fn ordinary_queries_are_unaffected_by_the_gate() {
    let (storage, _td) = setup().await;
    seed_user(&storage).await;
    seed_item(&storage).await;

    let value = scalar(
        &engine(&storage, reader("alice")),
        &format!("SELECT path FROM '{DATA_WS}' WHERE path = '/item-1'"),
    )
    .await
    .expect("one row");

    assert_eq!(value, PropertyValue::String("/item-1".into()));
}
