//! `QueryContext::create_engine` builds an engine per operation (function
//! transactions, `raisin.sql` inside a transaction). The prepared-statement
//! cache keys on the catalog's identity, so an engine built on a catalog of
//! its own made every statement a guaranteed miss — full analysis and logical
//! planning plus a dead cache entry, every time.

use super::QueryContext;
use crate::execution::types::{shared_http_client, ExecutionDependencies};
use raisin_binary::FilesystemBinaryStorage;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{BranchRepository, RepoScope, Storage, WorkspaceRepository};
use std::sync::Arc;

const TENANT: &str = "qc_cache_t";
const REPO: &str = "qc_cache_r";
const BRANCH: &str = "main";

#[tokio::test]
async fn function_transaction_statements_hit_the_prepared_statement_cache() {
    let dir = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path().join("db")).unwrap());
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "t", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new("ws".to_string()),
        )
        .await
        .unwrap();
    let deps = Arc::new(ExecutionDependencies {
        storage,
        binary_storage: Arc::new(FilesystemBinaryStorage::new(dir.path().join("bin"), None)),
        indexing_engine: None,
        hnsw_engine: None,
        http_client: shared_http_client(),
        ai_config_store: None,
        job_registry: None,
        job_data_store: None,
        lock_manager: None,
        secret_store: None,
        identity_repo: None,
        mount_content: None,
        schema_stats_cache: None,
    });
    let context = QueryContext::new(deps, TENANT.into(), REPO.into(), BRANCH.into(), None);
    let sql = "SELECT id FROM 'ws' WHERE path = '/qc-cache-probe'";

    // Two engines, as two operations of one function would build them: the
    // second must be answered from the entry the first put there. Retried
    // under eviction pressure (the cache is process-wide).
    for _ in 0..50 {
        let first = context.create_engine().await.unwrap();
        first.prepared_physical_plan(sql).await.unwrap();
        let second = context.create_engine().await.unwrap();
        let (_, cached) = second.prepared_physical_plan(sql).await.unwrap();
        if cached {
            return;
        }
    }
    panic!("a statement from a function transaction is never served from the cache");
}
