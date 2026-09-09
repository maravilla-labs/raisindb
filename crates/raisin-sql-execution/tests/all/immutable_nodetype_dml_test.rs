//! `NodeType.immutable` enforced through the actual SQL DML executor — the
//! path SQL DML and `psql` take, which bypasses `NodeService` entirely (see
//! CLAUDE.md, "Secrets & Encryption" / `crate::immutability` in
//! `raisin-rocksdb`). A rejection proven only at the storage-layer API
//! (`raisin-rocksdb/tests/all/immutable_nodetype_test.rs`) would not prove
//! the `psql` bypass is actually covered — this does.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_immutable_dml";
const REPO: &str = "r_immutable_dml";
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

fn create_test_catalog(workspaces: &[&str]) -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    for ws in workspaces {
        catalog.register_workspace(ws.to_string());
    }
    Arc::new(catalog)
}

async fn run_sql_count(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> usize {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut n = 0;
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        n += 1;
    }
    n
}

/// Run a statement and expect it to fail — either up front, or partway
/// through draining the row stream (DML errors surface lazily, the same way
/// `run_sql_count`'s row-level `.unwrap_or_else` above would panic on one).
async fn run_sql_expect_err(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> raisin_error::Error {
    match engine.execute(sql).await {
        Err(e) => e,
        Ok(mut stream) => {
            while let Some(row) = stream.next().await {
                if let Err(e) = row {
                    return e;
                }
            }
            panic!("SQL [{sql}] was expected to fail but succeeded");
        }
    }
}

async fn create_node_type(storage: &raisin_rocksdb::RocksDBStorage, json: serde_json::Value) {
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(json).expect("nodetype json"),
            CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");
}

#[tokio::test]
async fn sql_dml_update_rejected_on_immutable_nodetype() {
    let (storage, _td) = create_test_storage().await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("create workspace");

    create_node_type(
        &storage,
        serde_json::json!({ "name": "test:Ledger", "immutable": true }),
    )
    .await;
    create_node_type(&storage, serde_json::json!({ "name": "test:Plain" })).await;

    let catalog = create_test_catalog(&[WS]);
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(catalog)
    .with_auth(raisin_models::auth::AuthContext::system());

    // INSERT (create) onto an immutable type succeeds — immutability only
    // blocks a subsequent property change.
    run_sql_count(
        &engine,
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('entry-1','/entry-1','test:Ledger','{\"title\":\"a\"}'::JSONB)",
    )
    .await;

    // UPDATE that changes `properties` on the immutable node must be rejected
    // by the SQL DML executor itself — proving the write-layer enforcement in
    // `crate::immutability::reject_if_immutable` is reached from this path,
    // not only from the direct transaction/repository API.
    let err = run_sql_expect_err(
        &engine,
        "UPDATE items SET properties = '{\"title\":\"b\"}'::jsonb WHERE path = '/entry-1'",
    )
    .await;
    let message = err.to_string();
    assert!(
        message.contains("immutable"),
        "expected an immutability rejection, got: {message}"
    );

    // (The structural-only-update-is-allowed case — a move/rename with no
    // property change — is exercised at the storage-layer test instead:
    // `raisin-rocksdb/tests/all/immutable_nodetype_test.rs::move_allowed_on_immutable_node_when_properties_unchanged`.)

    // A plain (non-immutable) type continues to accept property UPDATEs.
    run_sql_count(
        &engine,
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('entry-2','/entry-2','test:Plain','{\"title\":\"a\"}'::JSONB)",
    )
    .await;
    run_sql_count(
        &engine,
        "UPDATE items SET properties = '{\"title\":\"b\"}'::jsonb WHERE path = '/entry-2'",
    )
    .await;
}
