//! Hierarchy as a compound-index column, end to end.
//!
//! A folder listing is normally written WITHOUT naming a node type:
//!
//!     SELECT ... WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 10
//!
//! Two things had to be true for that to use an index, and neither was:
//!
//!  1. `CHILD_OF` had to be expressible as a compound-index column, so hierarchy
//!     could lead a sorted index and leave the trailing column free for the
//!     ORDER BY. That is `__parent_path`.
//!  2. The index had to be FOUND. Compound indexes were loaded only when the
//!     WHERE clause carried a literal `node_type =`, so an index whose entire
//!     purpose is serving folder listings could never be found by one.

use futures::StreamExt;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::{Node, NodeType};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CreateNodeOptions, NodeRepository,
    NodeTypeRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_cih";
const REPO: &str = "r_cih";
const BRANCH: &str = "main";
const WS: &str = "ws";
const NODE_TYPE: &str = "test:Message";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

fn message_type() -> NodeType {
    NodeType {
        id: Some(NODE_TYPE.to_string()),
        name: NODE_TYPE.to_string(),
        strict: Some(false),
        allowed_children: vec!["*".to_string()],
        indexable: Some(true),
        created_at: Some(chrono::Utc::now()),
        compound_indexes: Some(vec![CompoundIndexDefinition {
            name: "folder_time".to_string(),
            columns: vec![
                CompoundIndexColumn {
                    property: "__parent_path".to_string(),
                    column_type: CompoundColumnType::String,
                    ascending: None,
                },
                CompoundIndexColumn {
                    property: "__created_at".to_string(),
                    column_type: CompoundColumnType::Timestamp,
                    ascending: None,
                },
            ],
            has_order_column: true,
        }]),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(false),
        index_types: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        is_mixin: None,
    }
}

fn node(id: &str, path: &str, parent: &str) -> Node {
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        parent: Some(parent.to_string()),
        node_type: NODE_TYPE.to_string(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

async fn setup() -> (
    QueryEngine<raisin_rocksdb::RocksDBStorage>,
    Arc<raisin_rocksdb::RocksDBStorage>,
    TempDir,
) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(tmp.path()).expect("storage"));

    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await;

    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            message_type(),
            CommitMetadata::system("seed"),
        )
        .await
        .expect("upsert node type");

    for (id, path, parent) in [
        ("a", "/a", "/"),
        ("m0", "/a/m0", "a"),
        ("m1", "/a/m1", "a"),
        ("m2", "/a/m2", "a"),
    ] {
        storage
            .nodes()
            .create(
                scope(),
                node(id, path, parent),
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create");
    }

    // BUILD THE INDEX, the way production does.
    //
    // A declaration is not a built index: the planner consults the persisted
    // build state and DECLINES anything that is not `Ready`, because a scan over
    // an empty or stale keyspace yields missing rows with nothing downstream to
    // catch it. Nothing had built it here — the node type was upserted and the
    // nodes created, but no job ran — so every query in this file planned as a
    // PrefixScan and the assertions below could never hold.
    //
    // `rebuild_indexes(.., IndexType::Compound)` is the real build path and it
    // is synchronous, which is what a test needs; `sweep_compound_index_builds`
    // only QUEUES work, and there is no worker here to pick it up.
    raisin_rocksdb::management::async_indexing::rebuild_indexes(
        &storage,
        TENANT,
        REPO,
        BRANCH,
        WS,
        raisin_storage::IndexType::Compound,
    )
    .await
    .expect("build the compound index");

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog));

    (engine, storage, tmp)
}

async fn explain(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> String {
    let mut stream = engine.execute(sql).await.expect("explain");
    let row = stream.next().await.expect("row").expect("decode");
    match row.columns.get("QUERY PLAN") {
        Some(raisin_models::nodes::properties::PropertyValue::String(p)) => p.clone(),
        other => panic!("unexpected EXPLAIN output: {other:?}"),
    }
}

/// The whole point: a bare folder listing, ordered, bounded — no node type named.
#[tokio::test]
async fn bare_child_of_order_by_uses_the_compound_index_and_elides_the_sort() {
    let (engine, _storage, _tmp) = setup().await;

    let plan = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') \
         ORDER BY created_at DESC LIMIT 3",
    )
    .await;

    assert!(
        plan.contains("CompoundIndexScan"),
        "a bare CHILD_OF + ORDER BY must find the (__parent_path, __created_at) \
         index with no node_type predicate; plan:\n{plan}"
    );
    assert!(
        !plan.contains("Sort") && !plan.contains("TopN"),
        "and the trailing order column must elide the sort; plan:\n{plan}"
    );
}

/// Naming the node type must keep working — that path loads only that type's
/// definitions rather than the whole branch.
#[tokio::test]
async fn naming_the_node_type_still_finds_the_index() {
    let (engine, _storage, _tmp) = setup().await;

    let plan = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') \
         AND node_type = 'test:Message' ORDER BY created_at DESC LIMIT 3",
    )
    .await;

    assert!(
        plan.contains("CompoundIndexScan"),
        "the typed form must still match; plan:\n{plan}"
    );
}

/// Results must be correct, not merely indexed: the rows come back and the
/// LIMIT is respected.
#[tokio::test]
async fn the_indexed_listing_returns_the_right_rows() {
    let (engine, _storage, _tmp) = setup().await;

    let mut stream = engine
        .execute("SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 2")
        .await
        .expect("query");
    let mut names = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("decode");
        if let Some(raisin_models::nodes::properties::PropertyValue::String(n)) = row
            .columns
            .iter()
            .find(|(k, _)| k.rsplit('.').next() == Some("name"))
            .map(|(_, v)| v.clone())
        {
            names.push(n);
        }
    }

    assert_eq!(
        names.len(),
        2,
        "LIMIT 2 must return two rows; got {names:?}"
    );
    assert!(
        names.iter().all(|n| n.starts_with('m')),
        "only the folder's children may be returned; got {names:?}"
    );
}
