//! End-to-end proof that `~` / `~*` / `!~` / `!~*` / `SIMILAR TO` and
//! `= ANY(...)` / `> ALL(...)` reach a real row through the full SQL stack
//! (parser -> analyzer -> planner -> executor), not just the unit tests in
//! `physical_plan::eval::regex_ops`.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_regex_quant";
const REPO: &str = "r_regex_quant";
const BRANCH: &str = "main";
const WS: &str = "items";

async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
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
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Item" }))
                .expect("nodetype json"),
            CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");
    (Arc::new(storage), temp_dir)
}

fn make_engine(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage,
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(raisin_models::auth::AuthContext::system())
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

async fn insert_node(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    id: &str,
    path: &str,
    props: &str,
) {
    run_sql_count(
        engine,
        &format!(
            "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
             ('{id}','{path}','test:Item','{props}'::JSONB)"
        ),
    )
    .await;
}

async fn seed(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    insert_node(engine, "i0", "/i0", r#"{"name":"Hello World","tag":"a"}"#).await;
    insert_node(engine, "i1", "/i1", r#"{"name":"goodbye","tag":"b"}"#).await;
    insert_node(engine, "i2", "/i2", r#"{"name":"Hello Again","tag":"c"}"#).await;
}

#[tokio::test]
async fn regex_operators_reach_the_executor() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = make_engine(storage);
    seed(&engine).await;

    // `~` case-sensitive match
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'name'::String ~ '^Hello'")
        )
        .await,
        2
    );

    // `~*` case-insensitive match
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'name'::String ~* '^hello'")
        )
        .await,
        2
    );

    // `!~` negated match
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'name'::String !~ '^Hello'")
        )
        .await,
        1
    );

    // `!~*` negated case-insensitive match
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'name'::String !~* '^hello'")
        )
        .await,
        1
    );

    // SIMILAR TO (anchored, `%` = any run)
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'name'::String SIMILAR TO 'Hello%'")
        )
        .await,
        2
    );
}

#[tokio::test]
async fn quantified_any_all_reach_the_executor() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = make_engine(storage);
    seed(&engine).await;

    // `= ANY(array)` over a literal text array
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'tag'::String = ANY(ARRAY['a', 'c'])")
        )
        .await,
        2
    );

    // `<> ALL(array)` — every element must differ
    assert_eq!(
        run_sql_count(
            &engine,
            &format!("SELECT * FROM {WS} WHERE properties->>'tag'::String <> ALL(ARRAY['a', 'b'])")
        )
        .await,
        1
    );
}

#[tokio::test]
async fn now_minus_interval_reaches_the_executor() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = make_engine(storage);

    let old_ts = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let recent_ts = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    insert_node(&engine, "old", "/old", &format!(r#"{{"ts":"{old_ts}"}}"#)).await;
    insert_node(
        &engine,
        "recent",
        "/recent",
        &format!(r#"{{"ts":"{recent_ts}"}}"#),
    )
    .await;

    // "posts older than N days": NOW() - INTERVAL '7 days'
    assert_eq!(
        run_sql_count(
            &engine,
            &format!(
                "SELECT * FROM {WS} WHERE properties->>'ts'::String < NOW() - INTERVAL '7 days'"
            )
        )
        .await,
        1
    );

    assert_eq!(
        run_sql_count(
            &engine,
            &format!(
                "SELECT * FROM {WS} WHERE properties->>'ts'::String > NOW() - INTERVAL '7 days'"
            )
        )
        .await,
        1
    );
}
