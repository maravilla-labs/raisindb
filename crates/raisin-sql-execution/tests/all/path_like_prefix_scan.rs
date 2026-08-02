//! Regression tests for `path LIKE 'prefix%'` scan correctness against the
//! RocksDB backend:
//!
//! 1. A pattern with a NAME prefix after the last slash (`/job/t-%`) must match
//!    nodes like `/job/t-000` (previously the executor treated the whole prefix
//!    as a parent path, found no node there, and returned zero rows).
//! 2. `path LIKE '/parent/%'` must NOT include the `/parent` row itself
//!    (previously the descendant traversal emitted the traversal root too).
//! 3. `path LIKE '/parent/%' AND node_type = '...'` must stay inside the
//!    subtree even when the property index wins the scan choice (previously the
//!    residual-filter combiner silently DROPPED hierarchy predicates, returning
//!    every node of that type across all subtrees).
//! 4. Same as (3) for an indexed/unindexed JSON property equality.
//! 5. Stale-index hygiene: after a property value changes, an equality query on
//!    the OLD value must not return the node; after DELETE, path LIKE scans and
//!    property scans must not resurrect it (also across a storage reopen).

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

// Scope constants must be UNIQUE to this module — see the note in
// `dml_index_and_predicate_maintenance.rs`. Compound-index definitions are
// memoised process-globally by `{tenant}:{repo}:{branch}` while each module owns
// its own TempDir storage, so a shared scope key lets one module's index
// declarations steer another module's plans into an empty keyspace.
const TENANT: &str = "t_path_like";
const REPO: &str = "r_path_like";
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

/// Reopen the SAME RocksDB files with fresh handles (no re-initialization —
/// the branch already exists on disk).
fn reopen_storage(temp_dir: &TempDir) -> Arc<raisin_rocksdb::RocksDBStorage> {
    Arc::new(raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("reopen storage"))
}

fn create_test_catalog(workspaces: &[&str]) -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    for ws in workspaces {
        catalog.register_workspace(ws.to_string());
    }
    Arc::new(catalog)
}

fn make_engine(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    QueryEngine::new(
        storage,
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(create_test_catalog(&[WS]))
    .with_auth(raisin_models::auth::AuthContext::system())
}

/// Run a statement and return the yielded `path` column values, sorted.
async fn run_sql_paths(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<String> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut paths = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        let path = row
            .columns
            .iter()
            .find(|(k, _)| k.as_str() == "path" || k.ends_with(".path"))
            .map(|(_, v)| match v {
                raisin_models::nodes::properties::PropertyValue::String(s) => s.clone(),
                other => format!("{other:?}"),
            })
            .unwrap_or_else(|| panic!("row without path column [{sql}]"));
        paths.push(path);
    }
    paths.sort();
    paths
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

async fn create_workspace(storage: &raisin_rocksdb::RocksDBStorage) {
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("create workspace");
}

/// Insert a node via SQL (goes through the transactional write path, which
/// maintains every index the scans rely on).
async fn insert_node(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    id: &str,
    path: &str,
    node_type: &str,
    props: &str,
) {
    run_sql_count(
        engine,
        &format!(
            "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
             ('{id}','{path}','{node_type}','{props}'::JSONB)"
        ),
    )
    .await;
}

/// Seed a two-subtree fixture:
///   /joba            (test:Folder)
///   /joba/t-000      (test:Item, status=held,   k=va)
///   /joba/t-001      (test:Item, status=held,   k=va)
///   /joba/other      (test:Item, status=held,   k=va)
///   /jobb            (test:Folder)
///   /jobb/t-000      (test:Item, status=held,   k=vb)
///   /jobb/t-001      (test:Item, status=held,   k=vb)
async fn seed_two_subtrees(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    insert_node(engine, "fa", "/joba", "test:Folder", "{}").await;
    insert_node(engine, "fb", "/jobb", "test:Folder", "{}").await;
    insert_node(
        engine,
        "a0",
        "/joba/t-000",
        "test:Item",
        r#"{"status":"held","k":"va"}"#,
    )
    .await;
    insert_node(
        engine,
        "a1",
        "/joba/t-001",
        "test:Item",
        r#"{"status":"held","k":"va"}"#,
    )
    .await;
    insert_node(
        engine,
        "a2",
        "/joba/other",
        "test:Item",
        r#"{"status":"held","k":"va"}"#,
    )
    .await;
    insert_node(
        engine,
        "b0",
        "/jobb/t-000",
        "test:Item",
        r#"{"status":"held","k":"vb"}"#,
    )
    .await;
    insert_node(
        engine,
        "b1",
        "/jobb/t-001",
        "test:Item",
        r#"{"status":"held","k":"vb"}"#,
    )
    .await;
}

async fn setup() -> (
    Arc<raisin_rocksdb::RocksDBStorage>,
    TempDir,
    QueryEngine<raisin_rocksdb::RocksDBStorage>,
) {
    let (storage, td) = create_test_storage().await;
    create_workspace(&storage).await;
    create_node_type(&storage, serde_json::json!({ "name": "test:Folder" })).await;
    // `k` and `status` are DECLARED with a Property index: symptoms must be
    // fixed for indexed properties (index-backed JSON predicates are the
    // supported configuration).
    create_node_type(
        &storage,
        serde_json::json!({
            "name": "test:Item",
            "properties": [
                { "name": "k", "type": "String", "index": ["Property"] },
                { "name": "status", "type": "String", "index": ["Property"] }
            ]
        }),
    )
    .await;
    // A node type whose properties declare NO index at all.
    create_node_type(&storage, serde_json::json!({ "name": "test:Plain" })).await;
    let engine = make_engine(storage.clone());
    (storage, td, engine)
}

/// (a) A LIKE pattern whose prefix ends in a NAME fragment (after the last
/// slash) must match by string prefix.
#[tokio::test]
async fn path_like_name_prefix_matches_nodes() {
    let (_storage, _td, engine) = setup().await;
    seed_two_subtrees(&engine).await;

    let paths = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE path LIKE '/joba/t-%'",
    )
    .await;
    assert_eq!(
        paths,
        vec!["/joba/t-000".to_string(), "/joba/t-001".to_string()],
        "name-prefix LIKE must match nodes sharing the string prefix"
    );

    // A top-level name prefix (no directory part beyond the root slash).
    let paths = run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/job%'").await;
    assert_eq!(
        paths.len(),
        7,
        "top-level name-prefix LIKE must match both subtrees (roots + children), got {paths:?}"
    );
}

/// (b) `LIKE '/parent/%'` must not include the parent row itself; a pattern
/// without the trailing slash separator ('/joba%') DOES match the parent.
#[tokio::test]
async fn path_like_directory_prefix_excludes_parent() {
    let (_storage, _td, engine) = setup().await;
    seed_two_subtrees(&engine).await;

    let paths = run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/joba/%'").await;
    assert_eq!(
        paths,
        vec![
            "/joba/other".to_string(),
            "/joba/t-000".to_string(),
            "/joba/t-001".to_string()
        ],
        "'/joba/%' must return the children only, not '/joba' itself"
    );

    let paths = run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/joba%'").await;
    assert_eq!(
        paths,
        vec![
            "/joba".to_string(),
            "/joba/other".to_string(),
            "/joba/t-000".to_string(),
            "/joba/t-001".to_string()
        ],
        "'/joba%' (no slash separator) matches the parent too"
    );
}

/// (c) Combining a path LIKE with a node_type equality must stay inside the
/// subtree even when the property index wins the scan choice.
#[tokio::test]
async fn path_like_with_node_type_stays_in_subtree() {
    let (_storage, _td, engine) = setup().await;
    seed_two_subtrees(&engine).await;

    let paths = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE path LIKE '/joba/%' AND node_type = 'test:Item'",
    )
    .await;
    assert_eq!(
        paths,
        vec![
            "/joba/other".to_string(),
            "/joba/t-000".to_string(),
            "/joba/t-001".to_string()
        ],
        "hierarchy predicate must not be dropped when the property index wins"
    );

    // Name-prefix + node_type: both restrictions must hold together.
    let paths = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE path LIKE '/joba/t-%' AND node_type = 'test:Item'",
    )
    .await;
    assert_eq!(
        paths,
        vec!["/joba/t-000".to_string(), "/joba/t-001".to_string()],
    );

    // Deterministic residual-drop repro: when a HIGHER-priority scan (NodeIdScan)
    // wins, the path LIKE must survive as a row-level filter. 'b0' lives in
    // /jobb, so this must return nothing.
    let paths = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE id = 'b0' AND path LIKE '/joba/%'",
    )
    .await;
    assert_eq!(
        paths,
        Vec::<String>::new(),
        "path LIKE must not be silently dropped when an id lookup wins the scan"
    );
}

/// (c) JSON-property variant: an INDEXED property equality AND a path LIKE.
#[tokio::test]
async fn path_like_with_indexed_property_stays_in_subtree() {
    let (_storage, _td, engine) = setup().await;
    seed_two_subtrees(&engine).await;

    // status='held' matches nodes in BOTH subtrees; the path LIKE must narrow it.
    let paths = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE path LIKE '/joba/%' AND properties->>'status' = 'held'",
    )
    .await;
    assert_eq!(
        paths,
        vec![
            "/joba/other".to_string(),
            "/joba/t-000".to_string(),
            "/joba/t-001".to_string()
        ],
        "path LIKE must be applied when a JSON property index wins the scan"
    );
}

/// (c) Unindexed-property variant: same query shape against a node type whose
/// property declares NO index. Whatever access path is chosen, the answer must
/// not silently include rows from other subtrees.
#[tokio::test]
async fn path_like_with_unindexed_property_stays_in_subtree() {
    let (_storage, _td, engine) = setup().await;
    insert_node(engine_ref(&engine), "pa", "/joba", "test:Folder", "{}").await;
    insert_node(engine_ref(&engine), "pb", "/jobb", "test:Folder", "{}").await;
    insert_node(
        engine_ref(&engine),
        "p1",
        "/joba/x",
        "test:Plain",
        r#"{"u":"same"}"#,
    )
    .await;
    insert_node(
        engine_ref(&engine),
        "p2",
        "/jobb/x",
        "test:Plain",
        r#"{"u":"same"}"#,
    )
    .await;

    let paths = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE path LIKE '/joba/%' AND properties->>'u' = 'same'",
    )
    .await;
    assert_eq!(
        paths,
        vec!["/joba/x".to_string()],
        "unindexed property + path LIKE must not leak other subtrees"
    );
}

fn engine_ref(
    e: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
) -> &QueryEngine<raisin_rocksdb::RocksDBStorage> {
    e
}

/// (d) Reproduction attempt for transient duplicates: heavy write churn with
/// scans interleaved after every write. Each physical node must appear exactly
/// once in every scan, at every point in time.
#[tokio::test]
async fn write_churn_never_produces_duplicate_rows() {
    let (_storage, _td, engine) = setup().await;
    insert_node(&engine, "root", "/job", "test:Folder", "{}").await;
    for i in 0..10 {
        insert_node(
            &engine,
            &format!("n{i}"),
            &format!("/job/t-{i:03}"),
            "test:Item",
            r#"{"status":"held","k":"v"}"#,
        )
        .await;
    }

    for round in 0..5 {
        for i in 0..10 {
            let status = if (round + i) % 2 == 0 {
                "held"
            } else {
                "confirmed"
            };
            run_sql_count(
                &engine,
                &format!(
                    "UPDATE items SET properties = '{{\"status\":\"{status}\",\"k\":\"v\"}}'::jsonb \
                     WHERE path = '/job/t-{i:03}'"
                ),
            )
            .await;

            // Scan immediately after the write — no duplicates, ever.
            let paths =
                run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/job/%'").await;
            let mut deduped = paths.clone();
            deduped.dedup();
            assert_eq!(
                paths, deduped,
                "duplicate rows after write round {round}, node {i}: {paths:?}"
            );
            assert_eq!(paths.len(), 10, "row count drifted: {paths:?}");

            let by_type = run_sql_paths(
                &engine,
                "SELECT path FROM items WHERE path LIKE '/job/%' AND node_type = 'test:Item'",
            )
            .await;
            let mut deduped = by_type.clone();
            deduped.dedup();
            assert_eq!(
                by_type, deduped,
                "duplicate rows in typed scan: {by_type:?}"
            );
            assert_eq!(by_type.len(), 10);
        }
    }

    // Delete every other node, scan after each delete.
    for i in (0..10).step_by(2) {
        run_sql_count(
            &engine,
            &format!("DELETE FROM items WHERE path = '/job/t-{i:03}'"),
        )
        .await;
        let paths = run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/job/%'").await;
        assert!(
            !paths.contains(&format!("/job/t-{i:03}")),
            "deleted node still visible: {paths:?}"
        );
        let mut deduped = paths.clone();
        deduped.dedup();
        assert_eq!(paths, deduped, "duplicates after delete: {paths:?}");
    }
}

/// (d) After a property VALUE changes, an equality query on the OLD value must
/// return nothing (no stale index hits), and repeated updates must not produce
/// duplicate rows for one physical node.
#[tokio::test]
async fn updated_and_deleted_nodes_do_not_linger() {
    let (storage, td, engine) = setup().await;
    seed_two_subtrees(&engine).await;

    // Flip /joba/t-000 from held -> confirmed several times (write churn).
    for status in ["confirmed", "held", "confirmed"] {
        run_sql_count(
            &engine,
            &format!(
                "UPDATE items SET properties = '{{\"status\":\"{status}\",\"k\":\"va\"}}'::jsonb \
                 WHERE path = '/joba/t-000'"
            ),
        )
        .await;
    }

    // Old value must not match the updated node.
    let held = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE properties->>'status' = 'held'",
    )
    .await;
    assert!(
        !held.contains(&"/joba/t-000".to_string()),
        "stale property value must not match after update, got {held:?}"
    );
    // ... and the other four are still held.
    assert_eq!(held.len(), 4, "remaining held nodes, got {held:?}");

    // SQL COUNT must agree.
    let mut stream = engine
        .execute("SELECT COUNT(*) AS n FROM items WHERE properties->>'status' = 'held'")
        .await
        .expect("count sql");
    let mut counted: Option<i64> = None;
    while let Some(row) = stream.next().await {
        let row = row.expect("count row");
        if let Some(raisin_models::nodes::properties::PropertyValue::Integer(n)) =
            row.columns.get("n")
        {
            counted = Some(*n);
        }
    }
    assert_eq!(
        counted,
        Some(4),
        "COUNT must not include stale old-value entries"
    );

    // The property INDEX itself must be clean (no orphaned old-value entries):
    // count index entries directly, bypassing all SQL-level filtering.
    {
        use raisin_storage::{PropertyIndexRepository, StorageScope};
        let index_count = storage
            .property_index()
            .count_by_property(
                StorageScope::new(TENANT, REPO, BRANCH, WS),
                "status",
                &raisin_models::nodes::properties::PropertyValue::String("held".to_string()),
                false,
            )
            .await
            .expect("index count");
        assert_eq!(
            index_count, 4,
            "property index must tombstone old-value entries on update"
        );
    }

    // No duplicates from write churn: each path appears exactly once.
    let scan = run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/joba/%'").await;
    assert_eq!(
        scan,
        vec![
            "/joba/other".to_string(),
            "/joba/t-000".to_string(),
            "/joba/t-001".to_string()
        ],
        "write churn must not produce duplicate rows"
    );

    // Delete a node; no scan may resurrect it.
    run_sql_count(&engine, "DELETE FROM items WHERE path = '/joba/t-001'").await;
    let scan = run_sql_paths(&engine, "SELECT path FROM items WHERE path LIKE '/joba/%'").await;
    assert_eq!(
        scan,
        vec!["/joba/other".to_string(), "/joba/t-000".to_string()],
        "deleted node must disappear from path LIKE scans"
    );
    let by_prop = run_sql_paths(
        &engine,
        "SELECT path FROM items WHERE properties->>'k' = 'va' AND path LIKE '/joba/%'",
    )
    .await;
    assert_eq!(
        by_prop,
        vec!["/joba/other".to_string(), "/joba/t-000".to_string()],
        "deleted node must disappear from property scans"
    );

    // Reopen the storage (fresh handles over the same RocksDB files) — deleted
    // nodes must stay gone (no orphaned index entries surviving restart).
    drop(engine);
    drop(storage);
    let storage2 = reopen_storage(&td);
    let engine2 = make_engine(storage2);
    let scan = run_sql_paths(&engine2, "SELECT path FROM items WHERE path LIKE '/joba/%'").await;
    assert_eq!(
        scan,
        vec!["/joba/other".to_string(), "/joba/t-000".to_string()],
        "deleted node must stay gone after reopen"
    );
    let held = run_sql_paths(
        &engine2,
        "SELECT path FROM items WHERE properties->>'status' = 'held'",
    )
    .await;
    assert!(
        !held.contains(&"/joba/t-000".to_string()),
        "stale property entries must not survive a reopen, got {held:?}"
    );
}
