//! `ORDER BY path LIMIT n` over a whole workspace (no path-prefix predicate).
//!
//! That shape plans as `PropertyOrderScan: __path`. It used to return ZERO rows:
//! the scan was a depth-first walk that started by loading the workspace root
//! node, which a workspace does not store. Where it did find a root it emitted
//! EDITORIAL order, not path order, so `path > $cursor` keyset paging skipped
//! and repeated rows. These tests pin byte-wise path order — including the
//! `/a-b` < `/a/x` case that tree order gets wrong — against workspaces with no
//! root node, with editorial order deliberately different from path order.

use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{CreateNodeOptions, DeleteNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "default";

type Rocks = raisin_rocksdb::RocksDBStorage;

async fn create_test_storage() -> (Arc<Rocks>, TempDir) {
    use raisin_storage::BranchRepository;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = Rocks::new(temp_dir.path()).expect("Failed to create RocksDB storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    (Arc::new(storage), temp_dir)
}

fn engine(storage: Arc<Rocks>) -> QueryEngine<Rocks> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage,
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

async fn create(storage: &Arc<Rocks>, id: &str, path: &str, parent: &str, node_type: &str) {
    let name = path.rsplit('/').next().unwrap().to_string();
    storage
        .nodes()
        .create(
            scope(),
            Node {
                id: id.to_string(),
                path: path.to_string(),
                name,
                parent: Some(parent.to_string()),
                node_type: node_type.to_string(),
                properties: HashMap::new(),
                ..Default::default()
            },
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to create node");
}

/// Created in REVERSE name order, so editorial (insertion) order is the
/// opposite of path order at every level. `/a-b` sorts between `/a` and
/// `/a/x` in path order, but after `/a`'s whole subtree in tree order.
async fn setup(storage: &Arc<Rocks>) {
    create(storage, "z", "/z", "/", "raisin:Folder").await;
    create(storage, "ab", "/a-b", "/", "studio:Page").await;
    create(storage, "a", "/a", "/", "raisin:Folder").await;
    create(storage, "ay", "/a/y", "a", "studio:Page").await;
    create(storage, "ax", "/a/x", "a", "studio:Page").await;
}

/// Every path, in byte-wise order.
const ALL: [&str; 5] = ["/a", "/a-b", "/a/x", "/a/y", "/z"];

async fn query_paths(engine: &QueryEngine<Rocks>, sql: &str) -> Vec<String> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        if let Some(PropertyValue::String(p)) = row.get("path") {
            out.push(p.to_string());
        }
    }
    out
}

async fn explain(engine: &QueryEngine<Rocks>, sql: &str) -> String {
    let mut stream = engine
        .execute(&format!("EXPLAIN {sql}"))
        .await
        .expect("explain should execute");
    let row = stream
        .next()
        .await
        .expect("explain should yield a row")
        .expect("explain row should decode");
    match row.columns.get("QUERY PLAN") {
        Some(PropertyValue::String(plan)) => plan.clone(),
        other => panic!("unexpected EXPLAIN output: {other:?}"),
    }
}

#[tokio::test]
async fn order_by_path_limit_uses_the_path_order_scan() {
    let (storage, _tmp) = create_test_storage().await;
    setup(&storage).await;
    let engine = engine(storage);

    let plan = explain(&engine, "SELECT path FROM default ORDER BY path LIMIT 2").await;
    assert!(
        plan.contains("PropertyOrderScan: __path"),
        "these tests must exercise the path order scan, got:\n{plan}"
    );
}

#[tokio::test]
async fn order_by_path_limit_returns_rows_in_path_order() {
    let (storage, _tmp) = create_test_storage().await;
    setup(&storage).await;
    let engine = engine(storage);

    assert_eq!(
        query_paths(&engine, "SELECT path FROM default ORDER BY path LIMIT 3").await,
        ALL[..3].to_vec()
    );
    assert_eq!(
        query_paths(&engine, "SELECT * FROM default ORDER BY path ASC LIMIT 100").await,
        ALL.to_vec()
    );
    // Same answer as the unbounded query, which sorts in memory.
    assert_eq!(
        query_paths(&engine, "SELECT path FROM default ORDER BY path").await,
        ALL.to_vec()
    );
}

#[tokio::test]
async fn order_by_path_desc_and_offset() {
    let (storage, _tmp) = create_test_storage().await;
    setup(&storage).await;
    let engine = engine(storage);

    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default ORDER BY path DESC LIMIT 3"
        )
        .await,
        vec!["/z", "/a/y", "/a/x"]
    );
    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default ORDER BY path LIMIT 2 OFFSET 2"
        )
        .await,
        vec!["/a/x", "/a/y"]
    );
    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default ORDER BY path DESC LIMIT 2 OFFSET 2"
        )
        .await,
        vec!["/a/x", "/a-b"]
    );
}

#[tokio::test]
async fn order_by_path_with_a_filter() {
    let (storage, _tmp) = create_test_storage().await;
    setup(&storage).await;
    let engine = engine(storage);

    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default WHERE node_type = 'studio:Page' ORDER BY path LIMIT 2"
        )
        .await,
        vec!["/a-b", "/a/x"]
    );
}

/// The CLI pull pattern: page through the whole workspace on a path cursor.
/// Every row exactly once, in order, both directions.
#[tokio::test]
async fn keyset_paging_on_path_visits_every_row_once() {
    let (storage, _tmp) = create_test_storage().await;
    setup(&storage).await;
    let engine = engine(storage);

    for (dir, op, expected) in [
        ("ASC", ">", ALL.to_vec()),
        ("DESC", "<", ALL.iter().rev().copied().collect::<Vec<_>>()),
    ] {
        let mut seen: Vec<String> = Vec::new();
        loop {
            let sql = match seen.last() {
                None => format!("SELECT path FROM default ORDER BY path {dir} LIMIT 2"),
                Some(last) => format!(
                    "SELECT path FROM default WHERE path {op} '{last}' ORDER BY path {dir} LIMIT 2"
                ),
            };
            let page = query_paths(&engine, &sql).await;
            if page.is_empty() {
                break;
            }
            assert!(page.len() <= 2, "{sql} returned {page:?}");
            seen.extend(page);
            assert!(
                seen.len() <= ALL.len(),
                "paging did not terminate: {seen:?}"
            );
        }
        assert_eq!(seen, expected, "keyset paging {dir}");
    }

    // A bound that is not itself a stored path seeks to the right place too.
    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default WHERE path > '/a-' ORDER BY path LIMIT 2"
        )
        .await,
        vec!["/a-b", "/a/x"]
    );
    // An inclusive bound is a filter, not a cursor, and keeps its row.
    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default WHERE path >= '/a/x' ORDER BY path LIMIT 2"
        )
        .await,
        vec!["/a/x", "/a/y"]
    );
}

/// A move tombstones the old path entry and a delete tombstones the node:
/// neither may surface, and the moved node appears once, at its new path.
#[tokio::test]
async fn moved_and_deleted_nodes_appear_only_where_they_are() {
    let (storage, _tmp) = create_test_storage().await;
    setup(&storage).await;
    storage
        .nodes()
        .move_node(scope(), "ab", "/m", None)
        .await
        .expect("move");
    storage
        .nodes()
        .delete(scope(), "z", DeleteNodeOptions::default())
        .await
        .expect("delete");
    let engine = engine(storage);

    assert_eq!(
        query_paths(&engine, "SELECT path FROM default ORDER BY path LIMIT 10").await,
        vec!["/a", "/a/x", "/a/y", "/m"]
    );
    assert_eq!(
        query_paths(
            &engine,
            "SELECT path FROM default ORDER BY path DESC LIMIT 10"
        )
        .await,
        vec!["/m", "/a/y", "/a/x", "/a"]
    );
}
