//! `IS DISTINCT FROM` / `IS NOT DISTINCT FROM` are standard SQL (SQL:1999)
//! null-safe comparisons. They were previously rejected by the analyzer with
//! "Unsupported expression: IsDistinctFrom", breaking functions that used them.
//! These tests pin the null-safe semantics, including the JSON-arrow form
//! `properties->>'k'::String IS DISTINCT FROM 'v'` from the original report.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
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

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
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
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Item" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (storage, temp_dir)
}

fn engine(storage: &Arc<raisin_rocksdb::RocksDBStorage>) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system())
}

async fn count(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> usize {
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

#[tokio::test]
async fn is_distinct_from_is_null_safe() {
    let (storage, _td) = setup().await;
    let e = engine(&storage);

    // /a visible=true, /b visible=false, /c has NO visible property (-> NULL).
    for (id, path, props) in [
        ("a", "/a", r#"{"visible":"true"}"#),
        ("b", "/b", r#"{"visible":"false"}"#),
        ("c", "/c", r#"{"other":"x"}"#),
    ] {
        count(
            &e,
            &format!(
                "INSERT INTO items (id, path, node_type, properties) VALUES \
                 ('{id}','{path}','test:Item','{props}'::JSONB)"
            ),
        )
        .await;
    }

    // IS DISTINCT FROM 'true' matches everything that is NOT exactly 'true',
    // INCLUDING the NULL row (/c) — that is the whole point vs plain `<>`.
    assert_eq!(
        count(
            &e,
            "SELECT path FROM items WHERE properties->>'visible'::String IS DISTINCT FROM 'true'",
        )
        .await,
        2,
        "IS DISTINCT FROM 'true' must match false + missing (null-safe)"
    );

    // IS NOT DISTINCT FROM 'true' is null-safe equality -> only /a.
    assert_eq!(
        count(
            &e,
            "SELECT path FROM items WHERE properties->>'visible'::String IS NOT DISTINCT FROM 'true'",
        )
        .await,
        1,
        "IS NOT DISTINCT FROM 'true' must match only the exact 'true' row"
    );

    // Null-vs-null: both missing -> NOT distinct (false for DISTINCT).
    assert_eq!(
        count(
            &e,
            "SELECT path FROM items WHERE properties->>'visible'::String IS DISTINCT FROM properties->>'nope'::String",
        )
        .await,
        2,
        "two NULLs are NOT distinct; only /a (true vs null) and /b (false vs null) differ"
    );
}
