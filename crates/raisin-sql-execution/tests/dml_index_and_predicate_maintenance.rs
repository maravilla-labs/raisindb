//! Regression tests for SQL DML correctness against the RocksDB backend:
//!
//! 1. JSON property equality combined with a `path`/`id` equality must match
//!    (regression for `JsonPropertyEq::to_expr`, which previously rebuilt the
//!    predicate as a containment check and dropped the key → zero rows).
//! 2. DML reports the real `affected_rows` count (0 when a guarded UPDATE matches
//!    nothing, 1 when it matches a row).
//! 3. Compound (multi-column) indexes are maintained on the transactional write
//!    path — INSERT populates them and UPDATE tombstones stale entries — so nodes
//!    written via SQL are visible to compound-index scans.

use futures::StreamExt;
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

/// Run a statement and return the number of rows the stream yielded.
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

/// Run a DML statement and return its reported `affected_rows` count.
async fn run_dml_affected(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> i64 {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut affected = 0;
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        if let Some(raisin_models::nodes::properties::PropertyValue::Integer(n)) =
            row.columns.get("affected_rows")
        {
            affected += *n;
        }
    }
    affected
}

/// Count entries in the `status_expiry` compound index for a given status value.
async fn count_compound(storage: &raisin_rocksdb::RocksDBStorage, status: &str) -> usize {
    use raisin_storage::{CompoundColumnValue, CompoundIndexRepository, StorageScope};
    storage
        .compound_index()
        .scan_compound_index(
            StorageScope::new(TENANT, REPO, BRANCH, WS),
            "status_expiry",
            &[CompoundColumnValue::String(status.to_string())],
            false,
            true,
            None,
        )
        .await
        .expect("compound scan")
        .len()
}

async fn read_seq(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) -> Option<String> {
    let mut stream = engine
        .execute("SELECT properties->>'seq'::String AS seq FROM items WHERE path = '/counter-a'")
        .await
        .expect("select seq");
    let mut out = None;
    while let Some(row) = stream.next().await {
        let row = row.expect("row");
        if let Some(v) = row.columns.get("seq") {
            out = Some(format!("{:?}", v));
        }
    }
    out
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
async fn dml_index_and_predicate_maintenance() {
    let (storage, _td) = create_test_storage().await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("create workspace");

    // A plain node type for the single-counter row, and a node type that declares
    // a compound index over (status, expires_at) for the indexed records.
    create_node_type(&storage, serde_json::json!({ "name": "test:Counter" })).await;
    create_node_type(
        &storage,
        serde_json::json!({
            "name": "test:Record",
            "compound_indexes": [{
                "name": "status_expiry",
                "has_order_column": true,
                "columns": [
                    { "property": "status", "column_type": "String" },
                    { "property": "expires_at", "column_type": "String" }
                ]
            }]
        }),
    )
    .await;

    let catalog = create_test_catalog(&[WS]);
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(catalog)
    .with_auth(raisin_models::auth::AuthContext::system());

    // Seed a counter row with seq=0 (a JSON number).
    run_sql_count(
        &engine,
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('c1','/counter-a','test:Counter','{\"value\":5,\"seq\":0}'::JSONB)",
    )
    .await;
    assert_eq!(read_seq(&engine).await, Some("String(\"0\")".to_string()));

    // A number-valued property is found via a string comparison (`->>` yields text).
    assert_eq!(
        run_sql_count(&engine, "SELECT path FROM items WHERE properties->>'seq' = '0'").await,
        1,
        "number-valued property must be matchable via a string literal"
    );

    // Conditional UPDATE guarded on a property value.
    run_sql_count(
        &engine,
        "UPDATE items SET properties = '{\"value\":4,\"seq\":1}'::jsonb \
         WHERE path = '/counter-a' AND properties->>'seq'::String = '0'",
    )
    .await;
    assert_eq!(read_seq(&engine).await, Some("String(\"1\")".to_string()));

    // affected_rows must reflect reality: a stale guard matches 0, a fresh one 1.
    let stale = run_dml_affected(
        &engine,
        "UPDATE items SET properties = '{\"seq\":99}'::jsonb \
         WHERE path = '/counter-a' AND properties->>'seq'::String = '0'",
    )
    .await;
    assert_eq!(stale, 0, "a guard matching no row must report 0 affected rows");
    let fresh = run_dml_affected(
        &engine,
        "UPDATE items SET properties = '{\"value\":4,\"seq\":1}'::jsonb \
         WHERE path = '/counter-a' AND properties->>'seq'::String = '1'",
    )
    .await;
    assert_eq!(fresh, 1, "a guard matching a row must report 1 affected row");

    // Regression for JsonPropertyEq::to_expr: when `path` wins the scan, the JSON
    // equality becomes a remaining row-level filter — it must still match.
    assert_eq!(
        run_sql_count(
            &engine,
            "SELECT path FROM items WHERE path = '/counter-a' AND properties->>'seq' = '1'",
        )
        .await,
        1,
        "path + JSON-eq (no cast) must match the row"
    );
    assert_eq!(
        run_sql_count(
            &engine,
            "SELECT path FROM items WHERE path = '/counter-a' AND properties->>'seq' = '0'",
        )
        .await,
        0,
        "path + JSON-eq with a non-matching value must return no rows"
    );
    // Cast form behaves the same for correctness.
    assert_eq!(
        run_sql_count(
            &engine,
            "SELECT path FROM items WHERE path = '/counter-a' AND properties->>'seq'::String = '1'",
        )
        .await,
        1,
        "path + JSON-eq (cast form) must match the row"
    );

    // Insert N indexed records via SQL; they must appear in the compound index.
    const N: usize = 50;
    for i in 0..N {
        run_sql_count(
            &engine,
            &format!(
                "INSERT INTO items (id, path, node_type, properties) VALUES \
                 ('rec{i}','/rec-{i}','test:Record','{{\"group\":\"/g1\",\"status\":\"open\",\"expires_at\":\"2026-01-01T00:00:00.000Z\"}}'::JSONB)",
            ),
        )
        .await;
    }
    assert_eq!(
        count_compound(&storage, "open").await,
        N,
        "SQL-inserted records must be present in the compound index"
    );

    // Changing the indexed value via SQL UPDATE must tombstone the old entry and
    // write the new one (no stale results).
    run_sql_count(
        &engine,
        "UPDATE items SET properties = '{\"group\":\"/g1\",\"status\":\"closed\",\"expires_at\":\"2026-01-01T00:00:00.000Z\"}'::jsonb WHERE path = '/rec-0'",
    )
    .await;
    assert_eq!(count_compound(&storage, "open").await, N - 1, "updated record must leave the old index value");
    assert_eq!(count_compound(&storage, "closed").await, 1, "updated record must enter the new index value");

    // A property-filtered DELETE removes the matching records.
    assert_eq!(run_sql_count(&engine, "SELECT path FROM items WHERE node_type = 'test:Record'").await, N);
    run_sql_count(
        &engine,
        "DELETE FROM items WHERE properties->>'group'::String = '/g1'",
    )
    .await;
    assert_eq!(
        run_sql_count(&engine, "SELECT path FROM items WHERE node_type = 'test:Record'").await,
        0,
        "property-filtered DELETE must remove the matching records"
    );
}
