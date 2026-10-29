//! Stack-budget regression tests.
//!
//! The server aborted ("thread 'tokio-rt-worker' has overflowed its stack") on a
//! SQL UPDATE and on an ordinary Sort > Project > Filter query. The plan was only
//! four operators deep: in an unoptimised build the poll frame of
//! `execute_plan` alone was ~154 KB, so a handful of nested operators exhausted
//! the 2 MiB a tokio worker (or any `std::thread`) gets by default.
//!
//! These tests run representative statements on a thread with a SMALL fixed
//! stack and a current-thread runtime, i.e. under the same budget as a worker.
//! A regression aborts the test process (stack overflow cannot be caught), which
//! is exactly the production failure.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, Storage, WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_stack_budget";
const REPO: &str = "r_stack_budget";
const BRANCH: &str = "main";
const WS: &str = "orders";

/// The budget the statements must fit in. tokio worker threads and
/// `std::thread` default to 2 MiB; the statements are run in half of it so the
/// test also leaves headroom for the callers above the engine (WebSocket and
/// HTTP handlers, the function host).
const STACK_BYTES: usize = 1024 * 1024;

/// `STACK_BYTES`, or `RAISIN_TEST_STACK_KB` for measuring how much a statement needs.
fn stack_bytes() -> usize {
    std::env::var("RAISIN_TEST_STACK_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(STACK_BYTES)
}

fn run_on_small_stack<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    std::thread::Builder::new()
        .name("small-stack".into())
        .stack_size(stack_bytes())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(f());
        })
        .unwrap()
        .join()
        .expect("statement panicked on a small stack");
}

async fn collect(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<raisin_sql_execution::physical_plan::Row> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        rows.push(row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}")));
    }
    rows
}

#[tokio::test(flavor = "multi_thread")]
async fn statements_fit_a_small_stack() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).unwrap());
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            raisin_storage::RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .unwrap();

    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Order" })).unwrap(),
            CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .unwrap();

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let catalog = Arc::new(catalog);
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(catalog)
    .with_auth(raisin_models::auth::AuthContext::system());

    for i in 0..30 {
        collect(
            &engine,
            &format!(
                "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
                 ('o{i}','/o-{i}','test:Order','{{\"status\":\"{}\",\"buyer\":\"b{}\",\"total\":{i}}}'::JSONB)",
                if i % 3 == 0 { "pending" } else { "paid" },
                i % 4
            ),
        )
        .await;
    }

    let engine = Arc::new(engine);
    let e = engine.clone();
    // Run everything inside one small-stack thread. The test thread blocks on it.
    let handle = tokio::task::spawn_blocking(move || {
        run_on_small_stack(move || async move {
            // Sort > Project > Filter > scan, the shape of the crash.
            let rows = collect(
                &e,
                &format!(
                    "SELECT path, properties->>'status'::String AS st FROM {WS} \
                     WHERE properties->>'status'::String IN ('paid','confirmed') \
                       AND properties->>'buyer'::String = 'b1' \
                     ORDER BY updated_at DESC LIMIT 5"
                ),
            )
            .await;
            assert!(!rows.is_empty());

            // TopN with a computed ordering key.
            let rows = collect(
                &e,
                &format!(
                    "SELECT path FROM {WS} WHERE node_type = 'test:Order' \
                     ORDER BY properties->>'total'::String DESC LIMIT 3"
                ),
            )
            .await;
            assert_eq!(rows.len(), 3);

            // UPDATE guarded on a property, the order status change.
            let rows = collect(
                &e,
                &format!(
                    "UPDATE {WS} SET properties = '{{\"status\":\"fulfilled\",\"buyer\":\"b1\",\"total\":1}}'::jsonb \
                     WHERE path = '/o-1' AND properties->>'status'::String IN ('paid','pending')"
                ),
            )
            .await;
            assert_eq!(rows.len(), 1);

            // Bulk UPDATE through the pure IN-list route (re-runs a SELECT inside).
            collect(
                &e,
                &format!(
                    "UPDATE {WS} SET properties = '{{\"status\":\"x\",\"buyer\":\"b9\",\"total\":2}}'::jsonb \
                     WHERE properties->>'status'::String IN ('pending','paid')"
                ),
            )
            .await;
        });
    });
    handle.await.unwrap();
}
