//! Verifies the SQL patterns we document for prev/next navigation and
//! pagination on hierarchical data:
//!
//! 1. Keyset (cursor) pagination by path: `path > $last ORDER BY path LIMIT n`
//! 2. Prev/next sibling by path: `path < $cur ... DESC LIMIT 1` / `path > $cur ... LIMIT 1`
//! 3. Keyset pagination by a property (blog-style published_at cursor)
//! 4. LIMIT/OFFSET paging

use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "default";

async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    use raisin_storage::BranchRepository;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path())
        .expect("Failed to create RocksDB storage");

    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;

    (Arc::new(storage), temp_dir)
}

fn engine(
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
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

async fn create_node(storage: &Arc<raisin_rocksdb::RocksDBStorage>, node: Node) {
    storage
        .nodes()
        .create(
            scope(),
            node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to create node");
}

/// /blog with posts post-1 .. post-5, each with a published_at property.
async fn setup_blog(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    create_node(
        storage,
        Node {
            id: "blog".to_string(),
            path: "/blog".to_string(),
            name: "blog".to_string(),
            parent: Some("/".to_string()),
            node_type: "raisin:Folder".to_string(),
            properties: HashMap::new(),
            ..Default::default()
        },
    )
    .await;

    for i in 1..=5 {
        let mut props = HashMap::new();
        props.insert(
            "published_at".to_string(),
            PropertyValue::String(format!("2026-01-0{}", i)),
        );
        create_node(
            storage,
            Node {
                id: format!("post{}", i),
                path: format!("/blog/post-{}", i),
                name: format!("post-{}", i),
                parent: Some("blog".to_string()),
                node_type: "studio:Page".to_string(),
                properties: props,
                ..Default::default()
            },
        )
        .await;
    }
}

async fn query_paths(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<String> {
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

/// Keyset pagination by path: page 1 then "after cursor" page 2.
#[tokio::test]
async fn test_keyset_pagination_by_path() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let page1 = query_paths(
        &engine,
        "SELECT path FROM default WHERE CHILD_OF('/blog') ORDER BY path LIMIT 2",
    )
    .await;
    assert_eq!(page1, vec!["/blog/post-1", "/blog/post-2"]);

    let page2 = query_paths(
        &engine,
        "SELECT path FROM default WHERE CHILD_OF('/blog') AND path > '/blog/post-2' ORDER BY path LIMIT 2",
    )
    .await;
    assert_eq!(page2, vec!["/blog/post-3", "/blog/post-4"]);
}

/// Prev/next sibling of a given node by path order.
#[tokio::test]
async fn test_prev_next_sibling_by_path() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let next = query_paths(
        &engine,
        "SELECT path FROM default WHERE CHILD_OF('/blog') AND path > '/blog/post-3' ORDER BY path ASC LIMIT 1",
    )
    .await;
    assert_eq!(next, vec!["/blog/post-4"], "next sibling of post-3");

    let prev = query_paths(
        &engine,
        "SELECT path FROM default WHERE CHILD_OF('/blog') AND path < '/blog/post-3' ORDER BY path DESC LIMIT 1",
    )
    .await;
    assert_eq!(prev, vec!["/blog/post-2"], "prev sibling of post-3");
}

/// Blog-style keyset pagination on a property cursor (published_at).
#[tokio::test]
async fn test_keyset_pagination_by_property() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let newest_two = query_paths(
        &engine,
        "SELECT path FROM default WHERE DESCENDANT_OF('/blog') \
         ORDER BY properties->>'published_at'::String DESC LIMIT 2",
    )
    .await;
    assert_eq!(newest_two, vec!["/blog/post-5", "/blog/post-4"]);

    let next_page = query_paths(
        &engine,
        "SELECT path FROM default WHERE DESCENDANT_OF('/blog') \
           AND properties->>'published_at'::String < '2026-01-04' \
         ORDER BY properties->>'published_at'::String DESC LIMIT 2",
    )
    .await;
    assert_eq!(next_page, vec!["/blog/post-3", "/blog/post-2"]);
}

/// Same but prev/next article by published_at (the blog "previous post /
/// next post" links).
#[tokio::test]
async fn test_prev_next_article_by_property() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    // Current article is post-3 (published 2026-01-03).
    let next = query_paths(
        &engine,
        "SELECT path FROM default WHERE DESCENDANT_OF('/blog') \
           AND properties->>'published_at'::String > '2026-01-03' \
         ORDER BY properties->>'published_at'::String ASC LIMIT 1",
    )
    .await;
    assert_eq!(next, vec!["/blog/post-4"]);

    let prev = query_paths(
        &engine,
        "SELECT path FROM default WHERE DESCENDANT_OF('/blog') \
           AND properties->>'published_at'::String < '2026-01-03' \
         ORDER BY properties->>'published_at'::String DESC LIMIT 1",
    )
    .await;
    assert_eq!(prev, vec!["/blog/post-2"]);
}

/// Classic LIMIT/OFFSET paging still works (documented as the simple option).
#[tokio::test]
async fn test_limit_offset_paging() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let page2 = query_paths(
        &engine,
        "SELECT path FROM default WHERE CHILD_OF('/blog') ORDER BY path LIMIT 2 OFFSET 2",
    )
    .await;
    assert_eq!(page2, vec!["/blog/post-3", "/blog/post-4"]);
}

/// CHILD_OF with no ORDER BY returns children in their natural (ordered
/// children) order — the editorial order maintained by the ordering system.
#[tokio::test]
async fn test_child_of_natural_order() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let children = query_paths(&engine, "SELECT path FROM default WHERE CHILD_OF('/blog')").await;
    assert_eq!(
        children,
        vec![
            "/blog/post-1",
            "/blog/post-2",
            "/blog/post-3",
            "/blog/post-4",
            "/blog/post-5"
        ],
        "creation order == ordered-children order here"
    );
}
