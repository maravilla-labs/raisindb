//! Editorial (fractional-index) order as a first-class SQL ordering.
//!
//! RaisinDB's manual child ordering lives in the `ORDERED_CHILDREN` index. These
//! tests pin that it is reachable from SQL:
//!
//!  - `__order` is selectable and matches what the storage layer reports;
//!  - `ORDER BY __order` reproduces editorial order (both directions), and
//!    survives a reorder;
//!  - `__order > $cursor` keyset-paginates a parent's children;
//!  - the `Sort` is elided when the scan already emits editorial order.

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{CreateNodeOptions, ListOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "menu";

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

fn node(id: &str, path: &str, parent: &str) -> Node {
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        parent: Some(parent.to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

/// `/menu` with `item-00` .. `item-{n-1}` created in order.
async fn seed(storage: &Arc<raisin_rocksdb::RocksDBStorage>, n: usize) -> String {
    create_node(storage, node("menu", "/menu", "/")).await;
    for i in 0..n {
        create_node(
            storage,
            node(
                &format!("item{i:02}"),
                &format!("/menu/item-{i:02}"),
                "menu",
            ),
        )
        .await;
    }
    "menu".to_string()
}

/// Run a query and collect one text column per row.
async fn query_column(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
    column: &str,
) -> Vec<String> {
    let mut stream = engine.execute(sql).await.expect("query should execute");
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("row should decode");
        let value = row
            .columns
            .iter()
            .find(|(key, _)| key.rsplit('.').next() == Some(column))
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| {
                panic!(
                    "column '{column}' missing; row keys: {:?}",
                    row.columns.keys().collect::<Vec<_>>()
                )
            });
        out.push(match value {
            raisin_models::nodes::properties::PropertyValue::String(s) => s,
            other => format!("{other:?}"),
        });
    }
    out
}

/// Run an `EXPLAIN` and return its plan text.
async fn explain(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> String {
    let mut stream = engine.execute(sql).await.expect("explain should execute");
    let row = stream
        .next()
        .await
        .expect("explain should yield a row")
        .expect("explain row should decode");
    match row.columns.get("QUERY PLAN") {
        Some(raisin_models::nodes::properties::PropertyValue::String(plan)) => plan.clone(),
        other => panic!("unexpected EXPLAIN output: {other:?}"),
    }
}

/// Editorial order straight from storage, as the expected baseline.
async fn storage_order(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    parent_id: &str,
) -> Vec<String> {
    storage
        .nodes()
        .list_by_parent(scope(), parent_id, ListOptions::for_api())
        .await
        .expect("list_by_parent")
        .into_iter()
        .map(|n| n.name)
        .collect()
}

#[tokio::test]
async fn order_column_is_selectable_and_matches_storage() {
    let (storage, _tmp) = create_test_storage().await;
    let parent_id = seed(&storage, 5).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu')",
        "name",
    )
    .await;
    assert_eq!(
        names,
        storage_order(&storage, &parent_id).await,
        "CHILD_OF must return editorial order"
    );

    let labels = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu')",
        "__order",
    )
    .await;
    assert_eq!(labels.len(), 5, "every row must carry an __order label");
    assert!(
        labels.iter().all(|l| !l.is_empty() && l != "Null"),
        "__order must be populated, got {labels:?}"
    );

    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(
        sorted, labels,
        "rows must arrive in ascending __order, got {labels:?}"
    );
}

#[tokio::test]
async fn order_by_order_reproduces_editorial_order_both_directions() {
    let (storage, _tmp) = create_test_storage().await;
    let parent_id = seed(&storage, 6).await;
    let engine = engine(storage.clone());

    let expected = storage_order(&storage, &parent_id).await;

    let asc = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order",
        "name",
    )
    .await;
    assert_eq!(asc, expected, "ORDER BY __order must match editorial order");

    let desc = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order DESC",
        "name",
    )
    .await;
    let mut reversed = expected.clone();
    reversed.reverse();
    assert_eq!(desc, reversed, "ORDER BY __order DESC must reverse it");
}

#[tokio::test]
async fn order_by_order_follows_a_reorder() {
    let (storage, _tmp) = create_test_storage().await;
    let parent_id = seed(&storage, 5).await;

    storage
        .nodes()
        .reorder_child(scope(), "/menu", "item-04", 0, Some("move"), Some("t"))
        .await
        .expect("reorder");

    let engine = engine(storage.clone());
    let names = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order",
        "name",
    )
    .await;

    assert_eq!(
        names.first().map(String::as_str),
        Some("item-04"),
        "the reordered child must lead, got {names:?}"
    );
    assert_eq!(
        names,
        storage_order(&storage, &parent_id).await,
        "SQL and storage must agree after a reorder"
    );
}

/// The point of exposing the label: keyset pagination over manual ordering.
#[tokio::test]
async fn order_supports_keyset_pagination() {
    let (storage, _tmp) = create_test_storage().await;
    let parent_id = seed(&storage, 12).await;
    let engine = engine(storage.clone());

    let expected = storage_order(&storage, &parent_id).await;
    let mut collected: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;

    for _ in 0..10 {
        let sql = match &cursor {
            None => "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') \
                     ORDER BY __order LIMIT 5"
                .to_string(),
            Some(c) => format!(
                "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') \
                 AND __order > '{c}' ORDER BY __order LIMIT 5"
            ),
        };

        let names = query_column(&engine, &sql, "name").await;
        if names.is_empty() {
            break;
        }
        let labels = query_column(&engine, &sql, "__order").await;
        assert!(names.len() <= 5, "LIMIT 5 returned {} rows", names.len());

        cursor = labels.last().cloned();
        collected.extend(names);
    }

    assert_eq!(
        collected, expected,
        "keyset pagination must cover every child exactly once, in editorial order"
    );
}

/// The scan already emits editorial order, so the planner must not add a Sort.
#[tokio::test]
async fn sort_is_elided_when_scan_claims_editorial_order() {
    let (storage, _tmp) = create_test_storage().await;
    seed(&storage, 3).await;
    let engine = engine(storage.clone());

    let plan = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order",
    )
    .await;

    assert!(
        !plan.contains("Sort"),
        "Sort should be elided for ORDER BY __order over a CHILD_OF scan; plan:\n{plan}"
    );

    // A sort the scan cannot satisfy must still be planned.
    let other = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY name",
    )
    .await;
    assert!(
        other.contains("Sort") || other.contains("TopN"),
        "ORDER BY name must still sort; plan:\n{other}"
    );
}

/// `__tree_order` is only known to tree traversals; a CHILD_OF scan reports NULL
/// rather than guessing. (Phase 4 populates it for subtree scans.)
#[tokio::test]
async fn tree_order_is_null_on_non_traversal_scans() {
    let (storage, _tmp) = create_test_storage().await;
    seed(&storage, 3).await;
    let engine = engine(storage.clone());

    let paths = query_column(
        &engine,
        "SELECT name, __tree_order FROM 'menu' WHERE CHILD_OF('/menu')",
        "__tree_order",
    )
    .await;
    assert!(
        paths.iter().all(|p| p == "Null"),
        "__tree_order must be NULL on a direct-children scan, got {paths:?}"
    );
}

/// The ordering columns appear in `SELECT *`, like every other `__`-prefixed
/// generated column (`__revision`, `__branch`, `__workspace`).
///
/// This is a deliberate consistency choice rather than an oversight: hiding
/// exactly these two while the others are visible would be arbitrary. It does
/// mean `SELECT *` returns two more columns than before — worth noting for
/// clients that index results positionally.
#[tokio::test]
async fn select_star_exposes_ordering_columns_like_other_generated_columns() {
    let (storage, _tmp) = create_test_storage().await;
    seed(&storage, 3).await;
    let engine = engine(storage.clone());

    let mut stream = engine
        .execute("SELECT * FROM 'menu' WHERE CHILD_OF('/menu')")
        .await
        .expect("query should execute");
    let row = stream
        .next()
        .await
        .expect("at least one row")
        .expect("row should decode");

    let columns: Vec<&str> = row
        .columns
        .keys()
        .map(|k| k.rsplit('.').next().unwrap_or(k))
        .collect();

    // Precondition: this test only says something if the sibling generated
    // columns really are visible in SELECT *.
    for existing in ["__revision", "__branch", "__workspace"] {
        assert!(
            columns.contains(&existing),
            "expected existing generated column {existing} in SELECT *; got {columns:?}"
        );
    }

    for added in ["__order", "__tree_order"] {
        assert!(
            columns.contains(&added),
            "{added} should follow the same convention; got {columns:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Subtree document order (`__tree_order`)
// ---------------------------------------------------------------------------

/// A three-level tree under `/menu`, created so creation order is editorial order.
async fn seed_tree(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    create_node(storage, node("menu", "/menu", "/")).await;
    for (id, path, parent) in [
        ("a", "/menu/a", "menu"),
        ("a1", "/menu/a/a1", "a"),
        ("a1x", "/menu/a/a1/a1x", "a1"),
        ("a2", "/menu/a/a2", "a"),
        ("b", "/menu/b", "menu"),
        ("b1", "/menu/b/b1", "b"),
        ("c", "/menu/c", "menu"),
    ] {
        create_node(storage, node(id, path, parent)).await;
    }
}

/// Document order for the descendants of `/menu` (the subtree root itself is
/// excluded from a `DESCENDANT_OF` match).
fn expected_descendants() -> Vec<&'static str> {
    vec!["a", "a1", "a1x", "a2", "b", "b1", "c"]
}

#[tokio::test]
async fn descendant_of_returns_document_order_with_tree_order() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage).await;
    let engine = engine(storage.clone());

    let sql = "SELECT name, __tree_order FROM 'menu' \
               WHERE DESCENDANT_OF('/menu') ORDER BY __tree_order";

    let names = query_column(&engine, sql, "name").await;
    assert_eq!(
        names,
        expected_descendants(),
        "DESCENDANT_OF ORDER BY __tree_order must be pre-order depth-first"
    );

    let paths = query_column(&engine, sql, "__tree_order").await;
    assert!(
        paths.iter().all(|p| p != "Null" && !p.is_empty()),
        "a subtree scan must populate __tree_order, got {paths:?}"
    );

    // The column must be self-consistently sorted — that is what makes it a
    // usable cursor.
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(sorted, paths, "rows must arrive in ascending __tree_order");
}

/// The payoff: keyset pagination over a whole subtree, not just one sibling set.
#[tokio::test]
async fn tree_order_supports_subtree_keyset_pagination() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage).await;
    let engine = engine(storage.clone());

    let mut collected: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;

    for _ in 0..10 {
        let sql = match &cursor {
            None => "SELECT name, __tree_order FROM 'menu' WHERE DESCENDANT_OF('/menu') \
                     ORDER BY __tree_order LIMIT 3"
                .to_string(),
            Some(c) => format!(
                "SELECT name, __tree_order FROM 'menu' WHERE DESCENDANT_OF('/menu') \
                 AND __tree_order > '{c}' ORDER BY __tree_order LIMIT 3"
            ),
        };

        let names = query_column(&engine, &sql, "name").await;
        if names.is_empty() {
            break;
        }
        assert!(names.len() <= 3, "LIMIT 3 returned {} rows", names.len());

        cursor = query_column(&engine, &sql, "__tree_order").await.pop();
        collected.extend(names);
    }

    assert_eq!(
        collected,
        expected_descendants(),
        "subtree keyset pagination must cover every descendant exactly once, in document order"
    );
}

/// A reorder at the top of the tree must move that node's whole subtree with it.
#[tokio::test]
async fn tree_order_follows_a_reorder_of_a_whole_subtree() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage).await;

    // Move 'b' (which has a child) to the front of /menu.
    storage
        .nodes()
        .reorder_child(scope(), "/menu", "b", 0, Some("move"), Some("t"))
        .await
        .expect("reorder");

    let engine = engine(storage.clone());
    let names = query_column(
        &engine,
        "SELECT name, __tree_order FROM 'menu' WHERE DESCENDANT_OF('/menu') \
         ORDER BY __tree_order",
        "name",
    )
    .await;

    assert_eq!(
        names,
        vec!["b", "b1", "a", "a1", "a1x", "a2", "c"],
        "the reordered subtree must move as a unit, keeping its child adjacent"
    );
}

/// `ORDER BY __tree_order` over a subtree scan needs no Sort; `DESC` does,
/// because reversing document order is not something the traversal can do cheaply.
#[tokio::test]
async fn subtree_sort_is_elided_ascending_only() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage).await;
    let engine = engine(storage.clone());

    let asc = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'menu' WHERE DESCENDANT_OF('/menu') ORDER BY __tree_order",
    )
    .await;
    assert!(
        !asc.contains("Sort"),
        "ascending __tree_order should be satisfied by the traversal; plan:\n{asc}"
    );

    let desc = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'menu' WHERE DESCENDANT_OF('/menu') ORDER BY __tree_order DESC",
    )
    .await;
    assert!(
        desc.contains("Sort") || desc.contains("TopN"),
        "descending __tree_order must still be sorted; plan:\n{desc}"
    );
}

/// `ORDER BY path` and `ORDER BY __tree_order` are NOT interchangeable, and the
/// difference is the whole point of the feature.
///
/// Both walk the tree parent-before-child, which makes them easy to confuse. But
/// they order *siblings* differently:
///
/// - `path` sorts siblings **alphabetically by name** — a filesystem listing.
/// - `__tree_order` sorts siblings by **editorial (drag-and-drop) order**.
///
/// They coincide only when the editorial order happens to be alphabetical, which
/// is exactly the case that hides the bug. This test forces them apart.
#[tokio::test]
async fn order_by_path_is_alphabetical_while_tree_order_is_editorial() {
    let (storage, _tmp) = create_test_storage().await;
    create_node(&storage, node("menu", "/menu", "/")).await;
    // Create in NON-alphabetical order, so editorial order != alphabetical.
    for (id, path) in [("c", "/menu/c"), ("a", "/menu/a"), ("b", "/menu/b")] {
        create_node(&storage, node(id, path, "menu")).await;
    }
    let engine = engine(storage.clone());

    let by_path = query_column(
        &engine,
        "SELECT name, path FROM 'menu' WHERE DESCENDANT_OF('/menu') ORDER BY path",
        "name",
    )
    .await;
    assert_eq!(
        by_path,
        vec!["a", "b", "c"],
        "ORDER BY path sorts by name and discards editorial order"
    );

    let by_tree_order = query_column(
        &engine,
        "SELECT name, __tree_order FROM 'menu' WHERE DESCENDANT_OF('/menu') \
         ORDER BY __tree_order",
        "name",
    )
    .await;
    assert_eq!(
        by_tree_order,
        vec!["c", "a", "b"],
        "ORDER BY __tree_order must preserve the order the children were placed in"
    );

    // And the documented default for a hierarchy scan is editorial order, so an
    // unordered query agrees with __tree_order rather than with path.
    let unordered = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/menu')",
        "name",
    )
    .await;
    assert_eq!(
        unordered, by_tree_order,
        "with no ORDER BY, a hierarchy scan returns editorial order"
    );
}
