//! Integration tests for the SQL lock / inventory functions
//! (`RAISIN_TRY_ACQUIRE`, `RAISIN_RELEASE`, `RAISIN_CLAIM`, ...).
//!
//! These exercise the full SQL pipeline: analyzer signature resolution → async
//! eval routing → ACL gate → the shared `LockManager` (in-process backend here).

use futures::StreamExt;
use raisin_locks::InProcessLockManager;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{BranchRepository, Storage};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";

async fn make_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(dir.path()).expect("rocksdb");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "tester", None, None, false, false)
        .await;
    (Arc::new(storage), dir)
}

/// Build an engine sharing one in-process lock manager. `auth = None` simulates
/// an unauthenticated caller (ACL should deny lock ops).
fn engine(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    auth: Option<AuthContext>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut e = QueryEngine::new(storage, TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(StaticCatalog::default_nodes_schema()))
        .with_lock_manager(Arc::new(InProcessLockManager::new()));
    if let Some(a) = auth {
        e = e.with_auth(a);
    }
    e
}

/// Run a single-row scalar SELECT and return the `column1` cell.
async fn scalar(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> PropertyValue {
    let mut stream = engine
        .execute_batch(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed [{sql}]: {e}"));
    let row = stream
        .next()
        .await
        .unwrap_or_else(|| panic!("no row [{sql}]"))
        .unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    row.get("column1")
        .cloned()
        .unwrap_or_else(|| panic!("no column1 [{sql}]"))
}

/// JSONB scalar results surface as a JSON string cell; parse it.
fn as_json(pv: PropertyValue) -> serde_json::Value {
    match pv {
        PropertyValue::String(s) => serde_json::from_str(&s).expect("valid json cell"),
        other => panic!("expected JSON string cell, got {other:?}"),
    }
}

#[tokio::test]
async fn try_acquire_release_and_contention() {
    let (storage, _dir) = make_storage().await;
    let eng = engine(storage, Some(AuthContext::system()));

    // First acquire wins and returns a fence token.
    let first = as_json(scalar(&eng, "SELECT raisin_try_acquire('seat:14A', 5000)").await);
    assert_eq!(first["acquired"], serde_json::json!(true));
    let token = first["token"].as_i64().expect("token");
    assert!(token > 0);

    // Second acquire on the held key loses the tie-breaker.
    let second = as_json(scalar(&eng, "SELECT raisin_try_acquire('seat:14A', 5000)").await);
    assert_eq!(second["acquired"], serde_json::json!(false));

    // Release with the correct token frees it.
    let released = scalar(&eng, &format!("SELECT raisin_release('seat:14A', {token})")).await;
    assert_eq!(released, PropertyValue::Boolean(true));

    // Now it can be reacquired.
    let again = as_json(scalar(&eng, "SELECT raisin_try_acquire('seat:14A', 5000)").await);
    assert_eq!(again["acquired"], serde_json::json!(true));
}

#[tokio::test]
async fn claim_never_oversells() {
    let (storage, _dir) = make_storage().await;
    let eng = engine(storage, Some(AuthContext::system()));

    let a = as_json(scalar(&eng, "SELECT raisin_claim('flight:1', 1, 2)").await);
    assert_eq!(a["claimed"], serde_json::json!(true));
    let b = as_json(scalar(&eng, "SELECT raisin_claim('flight:1', 1, 2)").await);
    assert_eq!(b["claimed"], serde_json::json!(true));
    // Pool exhausted.
    let c = as_json(scalar(&eng, "SELECT raisin_claim('flight:1', 1, 2)").await);
    assert_eq!(c["claimed"], serde_json::json!(false));
}

#[tokio::test]
async fn acl_denies_unauthenticated() {
    let (storage, _dir) = make_storage().await;
    let eng = engine(storage, None); // no auth context

    let err = eng
        .execute_batch("SELECT raisin_try_acquire('x', 1000)")
        .await
        .err()
        .expect("unauthenticated lock acquire must be denied");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("authentication") || msg.contains("forbidden") || msg.contains("anonymous"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn ttl_is_capped() {
    let (storage, _dir) = make_storage().await;
    let eng = engine(storage, Some(AuthContext::system()));

    // 10 minutes exceeds the 5-minute SQL cap.
    let res = eng
        .execute_batch("SELECT raisin_try_acquire('y', 600000)")
        .await;
    assert!(res.is_err(), "ttl above the cap must be rejected");
}
