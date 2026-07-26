//! GROUP BY over a JSON property extraction (`properties ->> 'key'`) must
//! return the actual distinct values as group keys — not NULL.
//!
//! Regression: `SELECT properties ->> 'status'::String AS status, COUNT(*) AS n
//! FROM ws WHERE ... GROUP BY properties ->> 'status'::String` returned rows
//! whose group key was NULL for every group (counts were fine, keys were lost),
//! while a plain SELECT of the same extraction returned correct values.
//!
//! Tested both with the property declared `index: [Property]` on the NodeType
//! and without any index, and with/without the `::String` cast and `AS` alias.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "items";

/// Set up storage with a workspace and a node type. When `indexed` is true the
/// `status` property is declared with a Property index.
async fn setup(indexed: bool) -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");

    let node_type = if indexed {
        serde_json::json!({
            "name": "test:Job",
            "properties": [
                { "name": "status", "type": "String", "index": ["Property"] }
            ]
        })
    } else {
        serde_json::json!({
            "name": "test:Job",
            "properties": [
                { "name": "status", "type": "String" }
            ]
        })
    };
    for nt in [node_type, serde_json::json!({ "name": "test:Container" })] {
        storage
            .node_types()
            .create(
                BranchScope::new(TENANT, REPO, BRANCH),
                serde_json::from_value(nt).expect("nt"),
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
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system())
}

async fn exec(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<HashMap<String, PropertyValue>> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        rows.push(row.columns.into_iter().collect());
    }
    rows
}

/// Seed 5 jobs under /jobs: 2 running, 2 done, 1 failed.
async fn seed(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    exec(
        engine,
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('jobs','/jobs','test:Container','{}'::JSONB)",
    )
    .await;
    for (id, path, status) in [
        ("j1", "/jobs/j1", "running"),
        ("j2", "/jobs/j2", "running"),
        ("j3", "/jobs/j3", "done"),
        ("j4", "/jobs/j4", "done"),
        ("j5", "/jobs/j5", "failed"),
    ] {
        exec(
            engine,
            &format!(
                "INSERT INTO items (id, path, node_type, properties) VALUES \
                 ('{id}','{path}','test:Job','{{\"status\":\"{status}\"}}'::JSONB)"
            ),
        )
        .await;
    }
}

/// Collect `status → count` from GROUP BY output rows.
///
/// NULL group keys surface as a missing column (Project drops NULL columns) —
/// map those to the string "<NULL>" so assertions show what happened.
fn group_counts(
    rows: &[HashMap<String, PropertyValue>],
    key_col: &str,
    count_col: &str,
) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for row in rows {
        let key = match row.get(key_col) {
            Some(PropertyValue::String(s)) => s.clone(),
            Some(other) => format!("{other:?}"),
            None => "<NULL>".to_string(),
        };
        let count = match row.get(count_col) {
            Some(PropertyValue::Integer(n)) => *n,
            Some(PropertyValue::Float(f)) => *f as i64,
            other => panic!("unexpected count value {other:?} in row {row:?}"),
        };
        *out.entry(key).or_insert(0) += count;
    }
    out
}

fn expected() -> HashMap<String, i64> {
    HashMap::from([
        ("running".to_string(), 2),
        ("done".to_string(), 2),
        ("failed".to_string(), 1),
    ])
}

async fn assert_group_by_variants(indexed: bool) {
    let (storage, _td) = setup(indexed).await;
    let e = engine(&storage);
    seed(&e).await;

    let label = if indexed { "indexed" } else { "unindexed" };

    // Sanity: plain SELECT of the extraction returns real values.
    let rows = exec(
        &e,
        "SELECT properties ->> 'status' AS status FROM items WHERE node_type = 'test:Job'",
    )
    .await;
    assert_eq!(rows.len(), 5, "[{label}] plain select row count");
    for row in &rows {
        assert!(
            matches!(row.get("status"), Some(PropertyValue::String(_))),
            "[{label}] plain SELECT must return the extracted value, got {row:?}"
        );
    }

    // 1. GROUP BY extraction, no cast, with alias.
    let rows = exec(
        &e,
        "SELECT properties ->> 'status' AS status, COUNT(*) AS n \
         FROM items WHERE node_type = 'test:Job' \
         GROUP BY properties ->> 'status'",
    )
    .await;
    assert_eq!(
        group_counts(&rows, "status", "n"),
        expected(),
        "[{label}] GROUP BY ->> (no cast, alias) returned wrong group keys: {rows:?}"
    );

    // 2. GROUP BY extraction with ::String cast + alias (the live report shape).
    let rows = exec(
        &e,
        "SELECT properties ->> 'status'::String AS status, COUNT(*) AS n \
         FROM items WHERE node_type = 'test:Job' \
         GROUP BY properties ->> 'status'::String",
    )
    .await;
    assert_eq!(
        group_counts(&rows, "status", "n"),
        expected(),
        "[{label}] GROUP BY ->> ::String (alias) returned wrong group keys: {rows:?}"
    );

    // 3. GROUP BY extraction with cast, no alias for the key column.
    let rows = exec(
        &e,
        "SELECT properties ->> 'status'::String, COUNT(*) AS n \
         FROM items WHERE node_type = 'test:Job' \
         GROUP BY properties ->> 'status'::String",
    )
    .await;
    // Without an alias the key column name is implementation-defined; find the
    // non-count column per row.
    let mut counts: HashMap<String, i64> = HashMap::new();
    for row in &rows {
        let count = match row.get("n") {
            Some(PropertyValue::Integer(n)) => *n,
            other => panic!("[{label}] unexpected count {other:?} in {row:?}"),
        };
        let key = row
            .iter()
            .filter(|(k, _)| *k != "n")
            .find_map(|(_, v)| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "<NULL>".to_string());
        *counts.entry(key).or_insert(0) += count;
    }
    assert_eq!(
        counts,
        expected(),
        "[{label}] GROUP BY ->> ::String (no alias) returned wrong group keys: {rows:?}"
    );

    // 4. Adding the WHERE path LIKE prefix from the live report must not change anything.
    let plain = exec(
        &e,
        "SELECT path FROM items WHERE path LIKE '/jobs/%' AND node_type = 'test:Job'",
    )
    .await;
    assert_eq!(
        plain.len(),
        5,
        "[{label}] plain SELECT with path LIKE + node_type filter must match all 5 rows: {plain:?}"
    );
    let rows = exec(
        &e,
        "SELECT properties ->> 'status'::String AS status, COUNT(*) AS n \
         FROM items WHERE path LIKE '/jobs/%' AND node_type = 'test:Job' \
         GROUP BY properties ->> 'status'::String",
    )
    .await;
    assert_eq!(
        group_counts(&rows, "status", "n"),
        expected(),
        "[{label}] GROUP BY with path LIKE + node_type filter returned wrong group keys: {rows:?}"
    );

    // Control: GROUP BY a plain column keeps working.
    let rows = exec(
        &e,
        "SELECT node_type, COUNT(*) AS n FROM items GROUP BY node_type",
    )
    .await;
    let counts = group_counts(&rows, "node_type", "n");
    assert_eq!(
        counts.get("test:Job"),
        Some(&5),
        "[{label}] GROUP BY plain column broken: {rows:?}"
    );
}

#[tokio::test]
async fn group_by_json_extraction_unindexed_property() {
    assert_group_by_variants(false).await;
}

#[tokio::test]
async fn group_by_json_extraction_indexed_property() {
    assert_group_by_variants(true).await;
}
