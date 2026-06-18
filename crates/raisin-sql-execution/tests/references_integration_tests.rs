//! Integration tests for the `REFERENCES('workspace:/path')` predicate.
//!
//! These exercise the full SQL pipeline against real RocksDB storage and lock
//! in the two fixes:
//!   1. The planner selects `ReferenceIndexScan` for `REFERENCES(...)`, so the
//!      predicate works WITHOUT `properties` being projected (it used to fall
//!      back to a row-eval post-filter that silently returned nothing).
//!   2. The reverse reference index is keyed by the target's stable node id, so
//!      a reference survives the target being MOVED to a new path.

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

/// Create an asset node and a page that references it (by stable id).
async fn setup_asset_and_referrer(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    create_node(storage, folder("assets", "/assets")).await;
    create_node(storage, folder("content", "/content")).await;

    create_node(
        storage,
        Node {
            id: "asset1".to_string(),
            path: "/assets/hero.png".to_string(),
            name: "hero.png".to_string(),
            parent: Some("assets".to_string()),
            node_type: "raisin:Asset".to_string(),
            properties: HashMap::new(),
            ..Default::default()
        },
    )
    .await;

    let mut props = HashMap::new();
    props.insert(
        "image".to_string(),
        PropertyValue::Reference(RaisinReference {
            id: "asset1".to_string(),
            workspace: WS.to_string(),
            path: "/assets/hero.png".to_string(),
        }),
    );
    create_node(
        storage,
        Node {
            id: "page1".to_string(),
            path: "/content/page1".to_string(),
            name: "page1".to_string(),
            parent: Some("content".to_string()),
            node_type: "raisin:Page".to_string(),
            properties: props,
            ..Default::default()
        },
    )
    .await;
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

/// The keystone regression: `SELECT path ... WHERE REFERENCES(...)` with NO
/// `properties` projection must still find the referrer (proves the index scan
/// is selected, not the properties-dependent row-eval post-filter).
#[tokio::test]
async fn test_references_without_properties_projection() {
    let (storage, _tmp) = create_test_storage().await;
    setup_asset_and_referrer(&storage).await;
    let engine = engine(storage);

    let paths = query_paths(
        &engine,
        "SELECT path FROM default WHERE REFERENCES('default:/assets/hero.png')",
    )
    .await;

    assert_eq!(paths, vec!["/content/page1".to_string()]);
}

/// The reference must survive the target being MOVED: the reverse index is
/// keyed by the asset's stable id, and the executor resolves the queried path
/// to that id, so the referrer is found at the NEW path and no longer at the
/// OLD one — with zero re-indexing of the referrer.
#[tokio::test]
async fn test_references_survives_target_move() {
    let (storage, _tmp) = create_test_storage().await;
    setup_asset_and_referrer(&storage).await;

    storage
        .nodes()
        .move_node(scope(), "asset1", "/assets/moved.png", None)
        .await
        .expect("move failed");

    let engine = engine(storage);

    // Found at the NEW path.
    let at_new = query_paths(
        &engine,
        "SELECT path FROM default WHERE REFERENCES('default:/assets/moved.png')",
    )
    .await;
    assert_eq!(at_new, vec!["/content/page1".to_string()]);

    // No longer found at the OLD path (nothing lives there anymore).
    let at_old = query_paths(
        &engine,
        "SELECT path FROM default WHERE REFERENCES('default:/assets/hero.png')",
    )
    .await;
    assert!(at_old.is_empty(), "old path should resolve to no target");
}

/// Cross-workspace: the target lives in a DIFFERENT workspace (`assets`) than
/// the FROM/source workspace (`default`) being scanned. The executor must
/// resolve the target path in the TARGET's workspace, and the reverse-index
/// scan must run in the source partition. This is the real Studio shape
/// (stories/events referencing assets).
#[tokio::test]
async fn test_references_cross_workspace() {
    let (storage, _tmp) = create_test_storage().await;

    // Asset folder + asset in the `assets` workspace.
    let assets_scope = StorageScope::new(TENANT, REPO, BRANCH, "assets");
    let opts = || CreateNodeOptions {
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        ..Default::default()
    };
    storage
        .nodes()
        .create(
            assets_scope,
            Node {
                id: "asset_x".to_string(),
                path: "/hero.png".to_string(),
                name: "hero.png".to_string(),
                parent: Some("/".to_string()),
                node_type: "raisin:Asset".to_string(),
                properties: HashMap::new(),
                ..Default::default()
            },
            opts(),
        )
        .await
        .expect("create asset");

    // Referrer page in the `default` workspace pointing at the asset.
    create_node(&storage, folder("content", "/content")).await;
    let mut props = HashMap::new();
    props.insert(
        "image".to_string(),
        PropertyValue::Reference(RaisinReference {
            id: "asset_x".to_string(),
            workspace: "assets".to_string(),
            path: "/hero.png".to_string(),
        }),
    );
    create_node(
        &storage,
        Node {
            id: "page_x".to_string(),
            path: "/content/page_x".to_string(),
            name: "page_x".to_string(),
            parent: Some("content".to_string()),
            node_type: "raisin:Page".to_string(),
            properties: props,
            ..Default::default()
        },
    )
    .await;

    let engine = engine(storage);
    let paths = query_paths(
        &engine,
        "SELECT path FROM default WHERE REFERENCES('assets:/hero.png')",
    )
    .await;

    assert_eq!(paths, vec!["/content/page_x".to_string()]);
}

/// A non-existent target resolves to no id → zero referrers (no error).
#[tokio::test]
async fn test_references_nonexistent_target_is_empty() {
    let (storage, _tmp) = create_test_storage().await;
    setup_asset_and_referrer(&storage).await;
    let engine = engine(storage);

    let paths = query_paths(
        &engine,
        "SELECT path FROM default WHERE REFERENCES('default:/assets/does-not-exist.png')",
    )
    .await;

    assert!(paths.is_empty());
}
