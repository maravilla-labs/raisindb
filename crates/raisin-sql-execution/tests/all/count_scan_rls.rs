//! `COUNT(*)` respects row-level security.
//!
//! The `CountScan` / `PropertyIndexCountScan` fast paths count storage keys
//! without materializing a node or passing it through RLS. That is a workspace
//! row-count leak (and, with a property filter, an existence oracle over indexed
//! properties) for any caller RLS does not wave through wholesale. These tests
//! pin the fix: a restricted caller's `COUNT(*)` counts only the rows it may
//! read, while a system caller still gets the fast, unfiltered count.

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
/// A workspace the restricted reader is allowed to read.
const OPEN_WS: &str = "open";
/// A workspace the restricted reader has NO grant on.
const SECRET_WS: &str = "secret";

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);

    for ws in [OPEN_WS, SECRET_WS] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .expect("workspace");
    }

    for nt in ["test:Item", "test:Secret"] {
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
    catalog.register_workspace(OPEN_WS.to_string());
    catalog.register_workspace(SECRET_WS.to_string());
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

async fn count(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> i64 {
    match scalar(engine, sql).await.expect("one count row") {
        PropertyValue::Integer(n) => n,
        other => panic!("COUNT(*) must be an integer, got {other:?}"),
    }
}

/// A reader whose only grant is read on the OPEN workspace. It holds NO grant
/// on SECRET, so RLS must deny every SECRET row — and the count of SECRET.
fn open_reader() -> AuthContext {
    AuthContext::for_user("reader").with_permissions(ResolvedPermissions {
        user_id: "reader".into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new("/**", vec![Operation::Read]).with_workspace(OPEN_WS)],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

async fn seed(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    // Three items in OPEN (two of type A, one of type B), two secrets in SECRET.
    let sys = engine(storage, AuthContext::system());
    run(
        &sys,
        &format!(
            "INSERT INTO '{OPEN_WS}' (id, path, node_type, properties) VALUES \
             ('o1','/o1','test:Item','{{\"kind\":\"a\"}}'::JSONB), \
             ('o2','/o2','test:Item','{{\"kind\":\"a\"}}'::JSONB), \
             ('o3','/o3','test:Item','{{\"kind\":\"b\"}}'::JSONB)"
        ),
    )
    .await;
    run(
        &sys,
        &format!(
            "INSERT INTO '{SECRET_WS}' (id, path, node_type, properties) VALUES \
             ('s1','/s1','test:Secret','{{\"pii\":\"x\"}}'::JSONB), \
             ('s2','/s2','test:Secret','{{\"pii\":\"y\"}}'::JSONB)"
        ),
    )
    .await;
}

#[tokio::test]
async fn system_count_is_unfiltered() {
    let (storage, _td) = setup().await;
    seed(&storage).await;
    let sys = engine(&storage, AuthContext::system());

    assert_eq!(
        count(&sys, &format!("SELECT COUNT(*) FROM '{SECRET_WS}'")).await,
        2
    );
    assert_eq!(
        count(&sys, &format!("SELECT COUNT(*) FROM '{OPEN_WS}'")).await,
        3
    );
}

#[tokio::test]
async fn restricted_count_of_a_forbidden_workspace_is_zero() {
    let (storage, _td) = setup().await;
    seed(&storage).await;
    let reader = engine(&storage, open_reader());

    // The whole point: without the RLS-aware count this returned 2 (the raw key
    // count) although the reader can read zero SECRET rows.
    assert_eq!(
        count(&reader, &format!("SELECT COUNT(*) FROM '{SECRET_WS}'")).await,
        0,
        "COUNT(*) leaked the row count of a workspace the caller cannot read"
    );
}

#[tokio::test]
async fn restricted_filtered_count_of_a_forbidden_workspace_is_zero() {
    let (storage, _td) = setup().await;
    seed(&storage).await;
    let reader = engine(&storage, open_reader());

    // Hits the PropertyIndexCountScan path (equality on the indexed node_type).
    // Without the fix this is an existence oracle: a non-zero answer would
    // reveal that a SECRET of that type exists.
    assert_eq!(
        count(
            &reader,
            &format!("SELECT COUNT(*) FROM '{SECRET_WS}' WHERE node_type = 'test:Secret'")
        )
        .await,
        0,
        "filtered COUNT(*) is an existence oracle over a forbidden workspace"
    );
}

#[tokio::test]
async fn restricted_count_of_an_allowed_workspace_is_full() {
    let (storage, _td) = setup().await;
    seed(&storage).await;
    let reader = engine(&storage, open_reader());

    // The reader may read all of OPEN, so both count shapes return the truth.
    assert_eq!(
        count(&reader, &format!("SELECT COUNT(*) FROM '{OPEN_WS}'")).await,
        3
    );
    assert_eq!(
        count(
            &reader,
            &format!("SELECT COUNT(*) FROM '{OPEN_WS}' WHERE node_type = 'test:Item'")
        )
        .await,
        3
    );
}

/// A reader scoped to a single path subtree counts only that subtree — the
/// count must agree with what an equivalent row SELECT would return, not the
/// raw workspace key count.
#[tokio::test]
async fn restricted_count_matches_visible_subset() {
    let (storage, _td) = setup().await;
    seed(&storage).await;

    let subtree_reader = AuthContext::for_user("sub").with_permissions(ResolvedPermissions {
        user_id: "sub".into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        // Only /o1 is readable in OPEN.
        permissions: vec![Permission::new("/o1", vec![Operation::Read]).with_workspace(OPEN_WS)],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    });
    let reader = engine(&storage, subtree_reader);

    assert_eq!(
        count(&reader, &format!("SELECT COUNT(*) FROM '{OPEN_WS}'")).await,
        1,
        "COUNT(*) must count only the rows the caller may read"
    );
}
