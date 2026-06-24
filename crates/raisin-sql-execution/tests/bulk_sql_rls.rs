//! Security regression: a complex/compound-WHERE SQL UPDATE routes through the
//! BULK update path. That path must enforce Row-Level Security against the
//! caller's `AuthContext` — a restricted user may modify rows they own and must
//! be blocked from rows they do not. This guards the async bulk-SQL executor,
//! which runs deferred bulk writes under the submitter's captured identity (NOT
//! system); the RLS enforcement it relies on lives in this write path.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, NodeTypeRepository, RepoScope, Storage, WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "items";

async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    (Arc::new(storage), temp_dir)
}

fn catalog() -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    Arc::new(catalog)
}

/// A non-admin user that may only read/update nodes whose `owner` property
/// matches their own id (the canonical differential-RLS shape).
fn owner_scoped_user(user: &str) -> AuthContext {
    AuthContext::for_user(user).with_permissions(ResolvedPermissions {
        user_id: user.to_string(),
        email: Some(format!("{user}@test.com")),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![
            Permission::new("/**", vec![Operation::Read, Operation::Update])
                .with_condition("node.owner == auth.user_id".to_string()),
        ],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth: AuthContext,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(catalog())
        .with_auth(auth)
}

/// Execute a statement, draining the stream; panics on any error.
async fn exec(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

/// Execute a statement that is expected to be denied; swallow any error so the
/// test can assert on the resulting (unchanged) state instead.
async fn try_exec(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    if let Ok(mut stream) = engine.execute(sql).await {
        while let Some(row) = stream.next().await {
            if row.is_err() {
                break;
            }
        }
    }
}

/// Read a node's `status` property via a (system) engine that sees every row.
async fn status(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    path: &str,
) -> Option<String> {
    let sql =
        format!("SELECT properties->>'status'::String AS status FROM items WHERE path = '{path}'");
    let mut stream = engine.execute(&sql).await.expect("status select");
    while let Some(row) = stream.next().await {
        let row = row.expect("status row");
        if let Some(PropertyValue::String(s)) = row.columns.get("status") {
            return Some(s.clone());
        }
    }
    None
}

#[tokio::test]
async fn bulk_update_enforces_rls_for_restricted_user() {
    let (storage, _td) = create_test_storage().await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("create workspace");
    storage
        .node_types()
        .create(
            raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Item" })).expect("nodetype"),
            raisin_storage::CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");

    // Admin (system) seeds two items owned by different users. System bypasses RLS.
    let admin = engine(&storage, AuthContext::system());
    exec(
        &admin,
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('a','/alice-item','test:Item','{\"owner\":\"alice\",\"status\":\"active\"}'::JSONB)",
    )
    .await;
    exec(
        &admin,
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('b','/bob-item','test:Item','{\"owner\":\"bob\",\"status\":\"active\"}'::JSONB)",
    )
    .await;

    // Restricted user "alice": owner-scoped read/update only.
    let alice = engine(&storage, owner_scoped_user("alice"));

    // ---- Permitted: alice updates the row she owns (compound WHERE -> bulk path).
    exec(
        &alice,
        "UPDATE items SET properties = properties || '{\"status\":\"updated-by-alice\"}'::jsonb \
         WHERE path = '/alice-item' AND properties->>'owner'::String = 'alice'",
    )
    .await;
    assert_eq!(
        status(&admin, "/alice-item").await.as_deref(),
        Some("updated-by-alice"),
        "alice must be able to update the row she owns"
    );

    // ---- Denied: alice tries to update bob's row (same compound/bulk shape).
    // Whether the bulk path returns 0 matches or errors closed, the row must be
    // untouched — RLS must not be silently bypassed on the async/bulk path.
    try_exec(
        &alice,
        "UPDATE items SET properties = properties || '{\"status\":\"hacked-by-alice\"}'::jsonb \
         WHERE path = '/bob-item' AND properties->>'owner'::String = 'bob'",
    )
    .await;
    assert_eq!(
        status(&admin, "/bob-item").await.as_deref(),
        Some("active"),
        "alice must NOT be able to update a row owned by bob"
    );

    // ---- Control: the EXACT same UPDATE succeeds for bob (the owner). This
    // proves alice's failure above was RLS denial, not a non-matching WHERE.
    let bob = engine(&storage, owner_scoped_user("bob"));
    exec(
        &bob,
        "UPDATE items SET properties = properties || '{\"status\":\"updated-by-bob\"}'::jsonb \
         WHERE path = '/bob-item' AND properties->>'owner'::String = 'bob'",
    )
    .await;
    assert_eq!(
        status(&admin, "/bob-item").await.as_deref(),
        Some("updated-by-bob"),
        "bob (the owner) must be able to update his own row via the same statement"
    );
}
