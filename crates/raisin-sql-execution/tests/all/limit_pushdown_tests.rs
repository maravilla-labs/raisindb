//! LIMIT must bound the scan, and must never truncate in the wrong order.
//!
//! Two defects motivated these tests, both invisible to the existing suites:
//!
//!  1. `set_scan_limit` overwrote the planner's exact limit with a 200_000
//!     constant, so `LIMIT 10` asked storage for 200_000 children. The pushdown
//!     that `CHILD_OF` planning computes was therefore inert.
//!  2. `plan_limit` planned the child of an `ORDER BY … LIMIT` with a context
//!     whose `order_by` was `None`. A scan then believed there was no ORDER BY,
//!     claimed its OWN order, bounded itself in that order, and the TopN above
//!     sorted the wrong k rows.
//!
//! (2) is a correctness bug: it returns wrong rows, not just slowly. The
//! existing coverage missed both because it only ever ran ordered queries over
//! fixtures smaller than the limit, where truncation cannot be observed.

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
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

/// `/menu` with children created in editorial order `item-00 .. item-{n-1}`,
/// but NAMED so that alphabetical order is the exact REVERSE of editorial order.
///
/// That inversion is what makes the ORDER BY bug observable: a scan that bounds
/// in editorial order and then sorts by name yields the alphabetically LAST
/// items, never the first.
async fn seed_inverted(storage: &Arc<raisin_rocksdb::RocksDBStorage>, n: usize) {
    create_node(storage, node("menu", "/menu", "/")).await;
    for i in 0..n {
        // Editorial position i ⇒ name descends as i ascends.
        let name = format!("item-{:02}", n - 1 - i);
        create_node(
            storage,
            node(&format!("item{i:02}"), &format!("/menu/{name}"), "menu"),
        )
        .await;
    }
}

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

/// The regression: `LIMIT 10` must reach the scan as 10, not as a 200_000
/// constant. A `CHILD_OF` scan with no ORDER BY emits editorial order, which is
/// the documented default, so nothing above it can reorder or discard rows and
/// the exact bound is safe.
#[tokio::test]
async fn child_of_limit_reaches_the_scan_exactly() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 50).await;
    let engine = engine(storage.clone());

    let plan = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'menu' WHERE CHILD_OF('/menu') LIMIT 10",
    )
    .await;

    assert!(
        plan.contains("limit=10"),
        "LIMIT 10 must be pushed into the PrefixScan as 10; plan:\n{plan}"
    );
    assert!(
        !plan.contains("200000"),
        "the scan must not be bounded by the SCAN_LIMIT_BUFFER constant; plan:\n{plan}"
    );
}

/// An unbounded query must stay unbounded — the pushdown must not invent a limit.
#[tokio::test]
async fn child_of_without_limit_is_not_bounded() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 20).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu')",
        "name",
    )
    .await;

    assert_eq!(
        names.len(),
        20,
        "every child must be returned without a LIMIT"
    );
}

/// Correctness: `ORDER BY name LIMIT k` must return the k alphabetically-first
/// children, NOT the first k in editorial order re-sorted among themselves.
///
/// The fixture inverts the two orderings, so the buggy plan returns the
/// alphabetically LAST k — a result that can never overlap the correct answer.
#[tokio::test]
async fn order_by_name_limit_returns_the_alphabetically_first_rows() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 50).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY name LIMIT 5",
        "name",
    )
    .await;

    assert_eq!(
        names,
        vec![
            "item-00".to_string(),
            "item-01".to_string(),
            "item-02".to_string(),
            "item-03".to_string(),
            "item-04".to_string(),
        ],
        "ORDER BY name LIMIT 5 must return the alphabetically first five"
    );
}

/// The same hazard in the other direction: `ORDER BY name DESC LIMIT k`.
#[tokio::test]
async fn order_by_name_desc_limit_returns_the_alphabetically_last_rows() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 50).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY name DESC LIMIT 3",
        "name",
    )
    .await;

    assert_eq!(
        names,
        vec![
            "item-49".to_string(),
            "item-48".to_string(),
            "item-47".to_string(),
        ],
        "ORDER BY name DESC LIMIT 3 must return the alphabetically last three"
    );
}

/// `ORDER BY __order LIMIT k` — the one ordering a CHILD_OF scan genuinely
/// satisfies — must still return the first k in editorial order, and should not
/// need a Sort above the scan.
#[tokio::test]
async fn order_by_editorial_order_limit_is_bounded_and_correct() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 30).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order LIMIT 4",
        "name",
    )
    .await;

    // Editorial order is creation order, and the fixture names descend as
    // editorial position ascends.
    assert_eq!(
        names,
        vec![
            "item-29".to_string(),
            "item-28".to_string(),
            "item-27".to_string(),
            "item-26".to_string(),
        ],
        "ORDER BY __order LIMIT 4 must return the first four in editorial order"
    );
}

/// A LIMIT larger than the child count must return everything, not pad or
/// truncate — the refill loop must terminate when the index is exhausted.
#[tokio::test]
async fn limit_larger_than_child_count_returns_all_rows() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 7).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') LIMIT 100",
        "name",
    )
    .await;

    assert_eq!(names.len(), 7, "LIMIT above the row count returns all rows");
}

/// Paging with LIMIT/OFFSET over a bounded scan must not drop or duplicate rows.
/// The scan is bounded to `limit + offset`, so an off-by-one in the refill loop
/// shows up here as a short or overlapping page.
#[tokio::test]
async fn limit_offset_pages_cover_every_child_exactly_once() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 25).await;
    let engine = engine(storage.clone());

    let mut seen = Vec::new();
    for offset in (0..25).step_by(5) {
        let page = query_column(
            &engine,
            &format!("SELECT name FROM 'menu' WHERE CHILD_OF('/menu') LIMIT 5 OFFSET {offset}"),
            "name",
        )
        .await;
        assert_eq!(page.len(), 5, "page at offset {offset} should be full");
        seen.extend(page);
    }

    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        25,
        "paging must cover all 25 children exactly once; got {} rows, {} unique",
        seen.len(),
        unique.len()
    );
}

// ---------------------------------------------------------------------------
// DESCENDANT_OF
//
// A subtree walk costs ~2 RocksDB seeks per node, so an unbounded walk under a
// LIMIT is far more expensive than the equivalent CHILD_OF. `DESCENDANT_OF`
// nevertheless refused to claim its own order unless the query said
// `ORDER BY __tree_order ASC` explicitly, while `CHILD_OF` accepted the
// no-ORDER-BY case — so `DESCENDANT_OF('/x') LIMIT 10` walked everything.
// ---------------------------------------------------------------------------

/// Two levels under `/tree`: parents in editorial (creation) order, each with
/// one child. Names DESCEND as editorial position ascends, so document order and
/// alphabetical order disagree everywhere.
///
/// Document order (pre-order DFS) is:
///   item-{n-1}, sub-{n-1}, item-{n-2}, sub-{n-2}, ...
/// Alphabetical is:
///   item-00 .. item-{n-1}, sub-00 .. sub-{n-1}   ('i' < 's')
async fn seed_tree(storage: &Arc<raisin_rocksdb::RocksDBStorage>, n: usize) {
    create_node(storage, node("tree", "/tree", "/")).await;
    for i in 0..n {
        let label = format!("{:02}", n - 1 - i);
        let parent_id = format!("p{i:02}");
        create_node(
            storage,
            node(&parent_id, &format!("/tree/item-{label}"), "tree"),
        )
        .await;
        create_node(
            storage,
            node(
                &format!("c{i:02}"),
                &format!("/tree/item-{label}/sub-{label}"),
                &parent_id,
            ),
        )
        .await;
    }
}

/// The regression: a bare `DESCENDANT_OF(..) LIMIT 10` must bound the walk.
#[tokio::test]
async fn descendant_of_limit_reaches_the_scan_exactly() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage, 40).await;
    let engine = engine(storage.clone());

    let plan = explain(
        &engine,
        "EXPLAIN SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree') LIMIT 10",
    )
    .await;

    assert!(
        plan.contains("limit=10"),
        "LIMIT 10 must bound the subtree walk; plan:\n{plan}"
    );
}

/// A bounded subtree walk must return the first k nodes in DOCUMENT order —
/// the same rows the unbounded query's first k would be.
#[tokio::test]
async fn descendant_of_limit_returns_the_document_order_prefix() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage, 20).await;
    let engine = engine(storage.clone());

    let all = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree')",
        "name",
    )
    .await;
    let bounded = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree') LIMIT 6",
        "name",
    )
    .await;

    assert_eq!(all.len(), 40, "20 parents + 20 children");
    assert_eq!(bounded.len(), 6, "LIMIT 6 must return exactly six rows");
    assert_eq!(
        bounded,
        all[..6].to_vec(),
        "the bounded walk must be a prefix of the unbounded one"
    );
}

/// Bounding must not leak into ordering: `ORDER BY name LIMIT k` over a subtree
/// must return the alphabetically first k, not the first k in document order.
#[tokio::test]
async fn descendant_of_order_by_name_limit_is_not_truncated_in_walk_order() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage, 20).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree') ORDER BY name LIMIT 4",
        "name",
    )
    .await;

    assert_eq!(
        names,
        vec![
            "item-00".to_string(),
            "item-01".to_string(),
            "item-02".to_string(),
            "item-03".to_string(),
        ],
        "ORDER BY name LIMIT 4 must return the alphabetically first four"
    );
}

/// `ORDER BY __tree_order LIMIT k` — the ordering the walk natively satisfies —
/// must stay correct now that the no-ORDER-BY case also claims that order.
#[tokio::test]
async fn descendant_of_order_by_tree_order_limit_matches_document_order() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage, 15).await;
    let engine = engine(storage.clone());

    let explicit = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree') ORDER BY __tree_order LIMIT 5",
        "name",
    )
    .await;
    let implicit = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree') LIMIT 5",
        "name",
    )
    .await;

    assert_eq!(explicit.len(), 5);
    assert_eq!(
        explicit, implicit,
        "an explicit ORDER BY __tree_order and no ORDER BY must agree — both are document order"
    );
}

/// A depth-bounded subtree query adds a residual depth filter, so the walk is
/// deliberately NOT bounded (the filter would eat the budget). It must still
/// return the right rows.
#[tokio::test]
async fn descendant_of_with_max_depth_is_still_correct() {
    let (storage, _tmp) = create_test_storage().await;
    seed_tree(&storage, 10).await;
    let engine = engine(storage.clone());

    let names = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE DESCENDANT_OF('/tree', 1) LIMIT 4",
        "name",
    )
    .await;

    assert_eq!(names.len(), 4, "LIMIT 4 must still return four rows");
    assert!(
        names.iter().all(|n| n.starts_with("item-")),
        "depth 1 must exclude the grandchildren; got {names:?}"
    );
}

/// Keyset pagination must stay bounded: the `__order > cursor` predicate is
/// absorbed into the index seek and dropped from the residual filter, so the
/// LIMIT still reaches the scan. If it were left as a residual filter the scan
/// would go unbounded and every page would cost a full folder read.
#[tokio::test]
async fn keyset_pagination_keeps_the_limit_pushed_down() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 40).await;
    let engine = engine(storage.clone());

    // Page 1: no cursor.
    let first = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order LIMIT 5",
        "__order",
    )
    .await;
    assert_eq!(first.len(), 5);
    let cursor = first.last().unwrap().clone();

    // Page 2: cursor form must still push the limit into the scan.
    let plan = explain(
        &engine,
        &format!(
            "EXPLAIN SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') \
             AND __order > '{cursor}' ORDER BY __order LIMIT 5"
        ),
    )
    .await;
    assert!(
        plan.contains("limit=5"),
        "keyset page must stay bounded; plan:\n{plan}"
    );
    assert!(
        !plan.contains("Filter"),
        "the cursor predicate must be absorbed into the seek, not left as a \
         residual Filter (a residual filter disables limit pushdown); plan:\n{plan}"
    );

    // And it must actually page correctly: no gaps, no repeats.
    let names_p1 = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order LIMIT 5",
        "name",
    )
    .await;
    let names_p2 = query_column(
        &engine,
        &format!(
            "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') AND __order > '{cursor}' \
             ORDER BY __order LIMIT 5"
        ),
        "name",
    )
    .await;
    let all = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order LIMIT 10",
        "name",
    )
    .await;
    let mut paged = names_p1.clone();
    paged.extend(names_p2.clone());
    assert_eq!(paged, all, "two keyset pages must equal one LIMIT 10");
}

/// DESCENDING keyset pagination — the newest-first inbox case.
///
/// The storage layer has always understood a backward cursor
/// (`OrderedScanStart::After` compares `candidate < label` when descending), but
/// the planner only ever lifted `__order > cursor`. A `__order < cursor` bound
/// was therefore left as a residual filter, which disabled limit pushdown and
/// made every newest-first page a full folder read.
#[tokio::test]
async fn descending_keyset_pagination_is_absorbed_and_bounded() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 40).await;
    let engine = engine(storage.clone());

    // Page 1, newest first.
    let cursors = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order DESC LIMIT 5",
        "__order",
    )
    .await;
    assert_eq!(cursors.len(), 5);
    let cursor = cursors.last().unwrap().clone();

    let plan = explain(
        &engine,
        &format!(
            "EXPLAIN SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') \
             AND __order < '{cursor}' ORDER BY __order DESC LIMIT 5"
        ),
    )
    .await;
    assert!(
        plan.contains("limit=5"),
        "descending keyset page must stay bounded; plan:\n{plan}"
    );
    assert!(
        !plan.contains("Filter"),
        "the `<` cursor must be absorbed into the seek, not left as a residual \
         Filter; plan:\n{plan}"
    );

    // Two descending pages must equal one descending LIMIT 10.
    let p1 = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order DESC LIMIT 5",
        "name",
    )
    .await;
    let p2 = query_column(
        &engine,
        &format!(
            "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') AND __order < '{cursor}' \
             ORDER BY __order DESC LIMIT 5"
        ),
        "name",
    )
    .await;
    let ten = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order DESC LIMIT 10",
        "name",
    )
    .await;

    let mut paged = p1.clone();
    paged.extend(p2.clone());
    assert_eq!(paged, ten, "two descending pages must equal one LIMIT 10");

    // And descending really is the reverse of ascending.
    let asc = query_column(
        &engine,
        "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order LIMIT 40",
        "name",
    )
    .await;
    let mut reversed = asc.clone();
    reversed.reverse();
    assert_eq!(ten, reversed[..10].to_vec(), "DESC must mirror ASC");
}

/// A cursor whose direction does NOT match the walk is a genuine range filter,
/// not a start position, and must NOT be swallowed by the seek.
#[tokio::test]
async fn mismatched_cursor_direction_is_kept_as_a_filter() {
    let (storage, _tmp) = create_test_storage().await;
    seed_inverted(&storage, 20).await;
    let engine = engine(storage.clone());

    let orders = query_column(
        &engine,
        "SELECT name, __order FROM 'menu' WHERE CHILD_OF('/menu') ORDER BY __order LIMIT 20",
        "__order",
    )
    .await;
    let midpoint = orders[10].clone();

    // `__order < x` under an ASCENDING walk is an upper bound. Absorbing it as a
    // cursor would return the rows at the END of the order instead of the start.
    let names = query_column(
        &engine,
        &format!(
            "SELECT name FROM 'menu' WHERE CHILD_OF('/menu') AND __order < '{midpoint}' \
             ORDER BY __order"
        ),
        "name",
    )
    .await;

    assert_eq!(
        names.len(),
        10,
        "an ascending walk with `__order < x` must return the FIRST ten rows, \
         not resume from x; got {names:?}"
    );
}
