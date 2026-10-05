//! A SELECT without FROM is evaluated directly, without a plan
//! (`engine/handlers/scalar.rs`), so every clause it does not honour is a
//! silently wrong answer. Phase 13b moved `execute` (functions, the
//! transaction query context) onto the same gate `execute_batch` used, and the
//! gate looked only at the FROM clause: `SELECT 1 WHERE false` returned a row,
//! and a UNION whose first query had no FROM dropped its right side.
//!
//! Every case runs through BOTH entry points.

use futures::StreamExt;
use raisin_locks::InProcessLockManager;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::RocksDBStorage;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{BranchRepository, Storage};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "scalar_t";
const REPO: &str = "scalar_r";
const BRANCH: &str = "main";

async fn engine() -> (QueryEngine<RocksDBStorage>, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let storage = RocksDBStorage::new(dir.path()).expect("rocksdb");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "tester", None, None, false, false)
        .await;
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("ws".to_string());
    let engine = QueryEngine::new(Arc::new(storage), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_lock_manager(Arc::new(InProcessLockManager::new()))
        .with_auth(AuthContext::system());
    (engine, dir)
}

#[derive(Clone, Copy, Debug)]
enum Entry {
    Execute,
    Batch,
}

const ENTRIES: [Entry; 2] = [Entry::Execute, Entry::Batch];

/// The number of rows `sql` returns, or the error it fails with.
async fn row_count(
    engine: &QueryEngine<RocksDBStorage>,
    entry: Entry,
    sql: &str,
) -> Result<usize, String> {
    let stream = match entry {
        Entry::Execute => engine.execute(sql).await,
        Entry::Batch => engine.execute_batch(sql).await,
    };
    let mut stream = stream.map_err(|e| e.to_string())?;
    let mut rows = 0;
    while let Some(row) = stream.next().await {
        row.map_err(|e| e.to_string())?;
        rows += 1;
    }
    Ok(rows)
}

#[tokio::test]
async fn select_without_from_where_false_returns_no_rows() {
    let (engine, _dir) = engine().await;
    for entry in ENTRIES {
        for (sql, expected) in [
            ("SELECT 1 AS x WHERE false", 0),
            ("SELECT 1 AS x WHERE 1 = 2", 0),
            ("SELECT 1 AS x WHERE 1 = NULL", 0),
            ("SELECT 1 AS x WHERE true", 1),
            ("SELECT 1 AS x WHERE 2 > 1", 1),
            ("SELECT 1 AS x", 1),
        ] {
            assert_eq!(
                row_count(&engine, entry, sql).await,
                Ok(expected),
                "[{entry:?}] {sql}"
            );
        }
    }
}

#[tokio::test]
async fn select_without_from_limit_zero_returns_no_rows() {
    let (engine, _dir) = engine().await;
    for entry in ENTRIES {
        for (sql, expected) in [
            ("SELECT 1 AS x LIMIT 0", 0),
            ("SELECT 1 AS x OFFSET 1", 0),
            ("SELECT 1 AS x LIMIT 5 OFFSET 1", 0),
            ("SELECT 1 AS x LIMIT 1", 1),
            ("SELECT 1 AS x LIMIT 1 OFFSET 0", 1),
            ("SELECT 1 AS x ORDER BY x", 1),
        ] {
            assert_eq!(
                row_count(&engine, entry, sql).await,
                Ok(expected),
                "[{entry:?}] {sql}"
            );
        }
    }
}

#[tokio::test]
async fn select_without_from_union_errors_instead_of_dropping_its_right_side() {
    let (engine, _dir) = engine().await;
    for entry in ENTRIES {
        for sql in [
            "SELECT 'all' AS name UNION ALL SELECT name FROM 'ws'",
            "SELECT 'all' AS name UNION SELECT 'other' AS name",
        ] {
            let err = row_count(&engine, entry, sql)
                .await
                .expect_err(&format!("[{entry:?}] {sql} must not answer one row"));
            assert!(err.contains("UNION"), "[{entry:?}] {sql}: {err}");
        }
    }
}

/// A side-effecting projection must not run for a row the query discards:
/// a claim filtered out by WHERE or LIMIT 0 leaves the pool untouched.
#[tokio::test]
async fn select_without_from_filtered_row_runs_no_side_effect() {
    let (engine, _dir) = engine().await;
    for entry in ENTRIES {
        let pool = format!("pool_{entry:?}");
        for sql in [
            format!("SELECT raisin_claim('{pool}', 1, 1) WHERE false"),
            format!("SELECT raisin_claim('{pool}', 1, 1) LIMIT 0"),
        ] {
            assert_eq!(row_count(&engine, entry, &sql).await, Ok(0), "{sql}");
        }
        // The single seat is still there.
        let mut stream = engine
            .execute_batch(&format!("SELECT raisin_claim('{pool}', 1, 1)"))
            .await
            .unwrap();
        let row = stream.next().await.expect("one row").unwrap();
        let Some(PropertyValue::String(cell)) = row.get("column1") else {
            panic!("[{entry:?}] expected a JSON cell, got {row:?}");
        };
        let claim: serde_json::Value = serde_json::from_str(cell).unwrap();
        assert_eq!(claim["claimed"], serde_json::json!(true), "[{entry:?}]");
    }
}
