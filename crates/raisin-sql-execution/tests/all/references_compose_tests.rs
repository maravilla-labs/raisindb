//! Regression tests for composing `REFERENCES(...)` with other predicates and
//! ordering — reported from a real blog implementation on RaisinDB:
//!
//! 1. `REFERENCES(...) AND DESCENDANT_OF(...)` silently returned ZERO rows
//!    (two index-driven predicates didn't compose).
//! 2. `ORDER BY created_at` returned all-null created_at values, making the
//!    ordering a silent no-op.

use futures::StreamExt;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "stories";
const TAGS_WS: &str = "tags";

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
    catalog.register_workspace(TAGS_WS.to_string());
    QueryEngine::new(
        storage,
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
}

fn scope(ws: &'static str) -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, ws)
}

async fn create_node(storage: &Arc<raisin_rocksdb::RocksDBStorage>, ws: &'static str, node: Node) {
    storage
        .nodes()
        .create(
            scope(ws),
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

fn folder(id: &str, path: &str) -> Node {
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: id.to_string(),
        parent: Some("/".to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

fn tag_ref(id: &str, path: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: id.to_string(),
        workspace: TAGS_WS.to_string(),
        path: path.to_string(),
    })
}

/// Blog shape: tags in their own `tags` workspace, story pages in the
/// `stories` workspace under per-site subtrees, each referencing a tag.
async fn setup_blog(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    // Tag in the tags workspace
    create_node(storage, TAGS_WS, folder("university", "/university")).await;
    create_node(
        storage,
        TAGS_WS,
        Node {
            id: "tag_data".to_string(),
            path: "/university/data".to_string(),
            name: "data".to_string(),
            parent: Some("university".to_string()),
            node_type: "studio:Tag".to_string(),
            properties: HashMap::new(),
            ..Default::default()
        },
    )
    .await;

    // Two sites in the stories workspace
    create_node(storage, WS, folder("siteA", "/siteA")).await;
    create_node(storage, WS, folder("siteB", "/siteB")).await;

    // Story in siteA that references the tag
    let mut props_a = HashMap::new();
    props_a.insert("tag".to_string(), tag_ref("tag_data", "/university/data"));
    props_a.insert(
        "published_at".to_string(),
        PropertyValue::String("2026-01-02".to_string()),
    );
    create_node(
        storage,
        WS,
        Node {
            id: "story_a".to_string(),
            path: "/siteA/story-a".to_string(),
            name: "story-a".to_string(),
            parent: Some("siteA".to_string()),
            node_type: "studio:Page".to_string(),
            properties: props_a,
            ..Default::default()
        },
    )
    .await;

    // Story in siteB that ALSO references the tag (must be excluded by DESCENDANT_OF('/siteA'))
    let mut props_b = HashMap::new();
    props_b.insert("tag".to_string(), tag_ref("tag_data", "/university/data"));
    create_node(
        storage,
        WS,
        Node {
            id: "story_b".to_string(),
            path: "/siteB/story-b".to_string(),
            name: "story-b".to_string(),
            parent: Some("siteB".to_string()),
            node_type: "studio:Page".to_string(),
            properties: props_b,
            ..Default::default()
        },
    )
    .await;

    // Story in siteA WITHOUT the tag (must be excluded by REFERENCES)
    let mut props_c = HashMap::new();
    props_c.insert(
        "published_at".to_string(),
        PropertyValue::String("2026-01-01".to_string()),
    );
    create_node(
        storage,
        WS,
        Node {
            id: "story_c".to_string(),
            path: "/siteA/story-c".to_string(),
            name: "story-c".to_string(),
            parent: Some("siteA".to_string()),
            node_type: "studio:Page".to_string(),
            properties: props_c,
            ..Default::default()
        },
    )
    .await;
}

async fn query_rows(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<raisin_sql_execution::Row> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        out.push(row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}")));
    }
    out
}

async fn query_paths(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<String> {
    query_rows(engine, sql)
        .await
        .iter()
        .filter_map(|row| match row.get("path") {
            Some(PropertyValue::String(p)) => Some(p.to_string()),
            _ => None,
        })
        .collect()
}

/// Reported bug 1: REFERENCES + DESCENDANT_OF returns zero rows.
#[tokio::test]
async fn test_references_and_descendant_of_compose() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let paths = query_paths(
        &engine,
        "SELECT path FROM stories WHERE REFERENCES('tags:/university/data') AND DESCENDANT_OF('/siteA')",
    )
    .await;

    assert_eq!(
        paths,
        vec!["/siteA/story-a".to_string()],
        "REFERENCES + DESCENDANT_OF must compose (only siteA story with the tag)"
    );
}

/// Same composition but with SELECT * (properties present in the row).
#[tokio::test]
async fn test_references_and_descendant_of_compose_select_star() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let paths = query_paths(
        &engine,
        "SELECT * FROM stories WHERE REFERENCES('tags:/university/data') AND DESCENDANT_OF('/siteA')",
    )
    .await;

    assert_eq!(paths, vec!["/siteA/story-a".to_string()]);
}

/// Sanity: each predicate works on its own.
#[tokio::test]
async fn test_each_predicate_alone() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let by_ref = query_paths(
        &engine,
        "SELECT path FROM stories WHERE REFERENCES('tags:/university/data') ORDER BY path",
    )
    .await;
    assert_eq!(
        by_ref,
        vec!["/siteA/story-a".to_string(), "/siteB/story-b".to_string()]
    );

    let by_desc = query_paths(
        &engine,
        "SELECT path FROM stories WHERE DESCENDANT_OF('/siteA') ORDER BY path",
    )
    .await;
    assert_eq!(
        by_desc,
        vec!["/siteA/story-a".to_string(), "/siteA/story-c".to_string()]
    );
}

/// Composition with node_type as well (the implementer's production query).
#[tokio::test]
async fn test_references_descendant_of_and_node_type() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let paths = query_paths(
        &engine,
        "SELECT id, path, name, node_type, properties FROM stories \
         WHERE REFERENCES('tags:/university/data') \
           AND DESCENDANT_OF('/siteA') \
           AND node_type = 'studio:Page' \
         LIMIT 200",
    )
    .await;

    assert_eq!(paths, vec!["/siteA/story-a".to_string()]);
}

/// Reported bug 2: ORDER BY created_at yields all-null created_at.
#[tokio::test]
async fn test_order_by_created_at_not_null() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let rows = query_rows(
        &engine,
        "SELECT path, created_at FROM stories WHERE REFERENCES('tags:/university/data') ORDER BY created_at DESC",
    )
    .await;

    assert!(!rows.is_empty());
    for row in &rows {
        let v = row.get("created_at");
        assert!(
            matches!(v, Some(PropertyValue::Date(_))),
            "created_at must be a non-null timestamp, got {:?}",
            v
        );
    }
}

/// SELECT * ... ORDER BY created_at must also carry non-null created_at.
#[tokio::test]
async fn test_select_star_order_by_created_at() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let rows = query_rows(
        &engine,
        "SELECT * FROM stories WHERE REFERENCES('tags:/university/data') ORDER BY created_at DESC LIMIT 4",
    )
    .await;

    assert!(!rows.is_empty());
    for row in &rows {
        let v = row.get("created_at");
        assert!(
            matches!(v, Some(PropertyValue::Date(_))),
            "created_at must be a non-null timestamp, got {:?}",
            v
        );
    }
}

/// Facet counts: COUNT(*) over REFERENCES (+ DESCENDANT_OF) must work.
#[tokio::test]
async fn test_count_with_references_and_descendant_of() {
    let (storage, _tmp) = create_test_storage().await;
    setup_blog(&storage).await;
    let engine = engine(storage);

    let rows = query_rows(
        &engine,
        "SELECT COUNT(*) AS cnt FROM stories WHERE REFERENCES('tags:/university/data') AND DESCENDANT_OF('/siteA')",
    )
    .await;
    assert_eq!(rows.len(), 1);
    let cnt = rows[0].get("cnt").cloned();
    assert!(
        matches!(cnt, Some(PropertyValue::Integer(1))),
        "expected count 1, got {:?}",
        cnt
    );
}
