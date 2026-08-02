// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Spatial predicate pushdown and the silent-empty trap, against a REAL engine.
//!
//! THE INVARIANT under test: *a predicate may be removed from the residual filter
//! ONLY IF the chosen access path is a proven-complete, exact answer for it.* In
//! every other case the predicate stays and the query is slower but correct. **A
//! spatial query must never return fewer rows than the truth.**
//!
//! It used to be violated in the way that produces silent wrong answers:
//! `has_spatial_index()` returned a hardcoded `true` ("the spatial_index CF is
//! always present" — a claim about schema, not about whether anything was ever
//! indexed for this property), and the planner then stripped `SpatialDWithin` from
//! the residual filter on the strength of it. On an unpopulated or stale index the
//! query returned ZERO ROWS with no fallback and no warning.
//!
//! # What this module covers, and what it deliberately does not
//!
//! These tests run the real `QueryEngine` over a real RocksDB and assert on
//! **result sets**, which is the only thing that can catch a silent-empty
//! regression. They exercise the FALLBACK path — the catalog reports `NotBuilt`
//! unless the backend supplies a state source — which is precisely the path that
//! used to return nothing.
//!
//! Plan *shape* assertions (which access path was chosen, whether the Sort was
//! elided, whether the LIMIT rode into the scan) need the planner's internals and
//! live in `src/physical_plan/planner/tests_spatial.rs`.
//!
//! # A note on the geometry-argument spelling
//!
//! All THREE spellings of the geometry argument analyse and return the same
//! rows today, proven by `both_geometry_spellings_return_the_same_rows`:
//!
//! * `CAST(properties->>'location' AS GEOMETRY)` — explicit, used by most tests
//!   in this module;
//! * `properties->>'location'` — the JSON extraction with no cast;
//! * `location` — the bare name, which is what the public reference and the
//!   `raisindb-sql` skill use throughout.
//!
//! This header previously stated that only the CAST form survived analysis and
//! that the other two failed. That was true when written and is not now. It is
//! called out because the claim was load-bearing: it made the documented
//! spelling look broken, and a stale "the docs are wrong" note is its own kind
//! of trap.
//!
//! The planner's `extract_geometry_source` has always handled all three
//! identically.

use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
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
const WS: &str = "shops";

/// Zurich HB, and three shops at known distances from it.
const CENTER_LON: f64 = 8.5402;
const CENTER_LAT: f64 = 47.3782;

async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path())
        .expect("Failed to create RocksDB storage");

    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;

    (Arc::new(storage), temp_dir)
}

/// A workspace, a node type and an engine that can run DML.
async fn setup() -> (
    Arc<raisin_rocksdb::RocksDBStorage>,
    QueryEngine<raisin_rocksdb::RocksDBStorage>,
    TempDir,
) {
    let (storage, tmp) = create_test_storage().await;
    let engine = engine_for(&storage).await;
    (storage, engine, tmp)
}

/// The workspace, node type and engine for an already-created storage.
async fn engine_for(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
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
            serde_json::from_value(serde_json::json!({ "name": "test:Shop" }))
                .expect("nodetype json"),
            CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(raisin_models::auth::AuthContext::system());

    engine
}

/// Insert a shop at `(lon, lat)` on `floor` **via SQL**.
///
/// Deliberately SQL rather than the repository API: the spec makes "geometry is
/// writable over SQL and maintains the index identically" a first-class
/// requirement, and a fixture that bypassed the SQL write path could not tell the
/// difference.
async fn insert_shop(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    name: &str,
    lon: f64,
    lat: f64,
    floor: &str,
) {
    let sql = format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
         ('{name}','/{name}','test:Shop', \
          '{{\"location\":{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}},\
             \"floor\":\"{floor}\"}}'::JSONB)"
    );
    let mut stream = engine
        .execute(&sql)
        .await
        .unwrap_or_else(|e| panic!("insert failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("insert row error: {e}"));
    }
}

/// Run a statement to completion, panicking on error.
async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

/// Run a query and collect the `name` column of every row, sorted.
async fn names(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Result<Vec<String>, String> {
    let mut stream = engine
        .execute(sql)
        .await
        .map_err(|e| format!("execute failed: {e}"))?;
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.map_err(|e| format!("row failed: {e}"))?;
        if let Some(PropertyValue::String(name)) = row.get("name") {
            out.push(name.clone());
        }
    }
    out.sort();
    Ok(out)
}

/// Offsets chosen so the three shops sit at roughly 25 m, 250 m and 25 km from
/// the centre — one inside every radius under test, one in the middle, one far.
async fn seed(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    // ~0.00033 deg latitude is ~37 m; ~0.0022 deg is ~245 m.
    insert_shop(engine, "near", CENTER_LON, CENTER_LAT + 0.000_33, "L2").await;
    insert_shop(engine, "mid", CENTER_LON, CENTER_LAT + 0.002_2, "L1").await;
    insert_shop(engine, "far", CENTER_LON, CENTER_LAT + 0.225, "L2").await;
}

/// THE regression. Whatever the index state is, `ST_DWITHIN` must return the
/// rows that are genuinely within the radius — never an empty set because the
/// index happened to be unpopulated.
#[tokio::test]
async fn st_dwithin_returns_the_right_rows_without_a_populated_index() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let sql = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 100)"
    );
    let rows = names(&engine, &sql).await.expect("query failed");
    assert_eq!(
        rows,
        vec!["near".to_string()],
        "a 100m ST_DWITHIN must return exactly the near shop; an empty result here \
         is the silent-empty regression"
    );
}

/// The radius window used to be silently ~4.8 m - 39 km: outside it, zero rows.
/// Sub-metre through continental must all give the truth.
#[tokio::test]
async fn every_radius_scale_returns_the_truth() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    // (radius_meters, expected names)
    let cases: Vec<(f64, Vec<&str>)> = vec![
        (0.5, vec![]),
        (1.0, vec![]),
        (100.0, vec!["near"]),
        (500.0, vec!["mid", "near"]),
        (50_000.0, vec!["far", "mid", "near"]),
        (500_000.0, vec!["far", "mid", "near"]),
    ];

    for (radius, expected) in cases {
        let sql = format!(
            "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
             ST_POINT({CENTER_LON}, {CENTER_LAT}), {radius})"
        );
        let rows = names(&engine, &sql).await.expect("query failed");
        let expected: Vec<String> = expected.into_iter().map(String::from).collect();
        assert_eq!(rows, expected, "wrong rows at radius {radius}m");
    }
}

/// Reversed argument order is the same predicate. It used to fall off the index
/// path entirely; it must at minimum still return the right rows.
#[tokio::test]
async fn reversed_argument_order_returns_the_same_rows() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let forward = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
    );
    let reversed = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(ST_POINT({CENTER_LON}, {CENTER_LAT}), \
         CAST(properties->>'location' AS GEOMETRY), 500)"
    );
    assert_eq!(
        names(&engine, &forward).await.expect("forward failed"),
        names(&engine, &reversed).await.expect("reversed failed"),
    );
}

/// `ST_DISTANCE(...) < r` is the same access path as `ST_DWITHIN(..., r)` and must
/// agree with it — and `>` (an anti-range with no index path) must return the
/// complement, not be silently dropped.
#[tokio::test]
async fn st_distance_comparisons_agree_with_st_dwithin() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let dwithin = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
    );
    let lt = format!(
        "SELECT name FROM '{WS}' WHERE ST_DISTANCE(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT})) < 500"
    );
    let gt = format!(
        "SELECT name FROM '{WS}' WHERE ST_DISTANCE(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT})) > 500"
    );

    let within = names(&engine, &dwithin).await.expect("dwithin failed");
    assert_eq!(names(&engine, &lt).await.expect("lt failed"), within);
    assert_eq!(
        names(&engine, &gt).await.expect("gt failed"),
        vec!["far".to_string()],
        "`ST_DISTANCE > r` has no index path and must not be dropped"
    );
}

/// Composition. A spatial predicate ANDed with a property equality (the floor /
/// level filter) must intersect, not replace. Dropping either side is the
/// `path LIKE '/a/%' AND node_type = 'X'` class of bug.
#[tokio::test]
async fn spatial_composes_with_a_floor_filter() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let sql = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 500) \
         AND properties->>'floor'::String = 'L1'"
    );
    let rows = names(&engine, &sql).await.expect("query failed");
    assert_eq!(
        rows,
        vec!["mid".to_string()],
        "the floor predicate must still apply after the spatial one"
    );

    // And the other way round: a floor with no shop in range yields nothing,
    // rather than every shop on that floor.
    let sql = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 100) \
         AND properties->>'floor'::String = 'L1'"
    );
    assert!(names(&engine, &sql).await.expect("query failed").is_empty());
}

/// A CONTROL for the test below: `ORDER BY <plain column> LIMIT k` works.
#[tokio::test]
async fn order_by_a_plain_column_works() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let mut stream = engine
        .execute(&format!("SELECT name FROM '{WS}' ORDER BY name LIMIT 2"))
        .await
        .expect("execute failed");
    let mut ordered = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("row failed");
        if let Some(PropertyValue::String(name)) = row.get("name") {
            ordered.push(name.clone());
        }
    }
    assert_eq!(ordered, vec!["far".to_string(), "mid".to_string()]);
}

/// `ORDER BY ST_DISTANCE(...) LIMIT k` — the canonical nearest-neighbour query —
/// must be ordered nearest-first.
///
/// # Formerly ignored, and why the original diagnosis was wrong
///
/// This test was `#[ignore]`d as "pre-existing: `ORDER BY <computed expression>
/// LIMIT k` returns zero rows in the TopN path". It was not a `TopN` defect and
/// the rows were never missing: `try_plan_spatial_knn` looked THROUGH the
/// `Project` above the scan to find it and then returned the bare
/// `SpatialKnnScan`, dropping the projection. A scan emits fully qualified
/// column names (`shops.name`, see `node_to_row`'s "Column Naming" section) and
/// the `Project` is what renames them to what the SELECT list asked for — so the
/// query returned the right k rows, in the right order, under a `shops.name`
/// column. `names()` below collects `row.get("name")`, which found nothing and
/// looked exactly like an empty result set.
///
/// Fixed in `plan_dispatch/spatial_knn.rs` by putting the `Project` back on top;
/// pinned at the planner level by
/// `tests_spatial::knn_keeps_the_projection_it_looked_through` and end to end by
/// `raisin-server`'s `spatial_query_test`.
#[tokio::test]
async fn order_by_st_distance_orders_by_distance() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let sql = format!(
        "SELECT name FROM '{WS}' \
         ORDER BY ST_DISTANCE(CAST(properties->>'location' AS GEOMETRY), ST_POINT({CENTER_LON}, {CENTER_LAT})) \
         LIMIT 2"
    );
    let mut stream = engine.execute(&sql).await.expect("execute failed");
    let mut ordered = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("row failed");
        if let Some(PropertyValue::String(name)) = row.get("name") {
            ordered.push(name.clone());
        }
    }
    assert_eq!(
        ordered,
        vec!["near".to_string(), "mid".to_string()],
        "nearest-first ordering must survive whichever access path was chosen"
    );
}

/// A moved geometry must match at its NEW position and NOT at its old one. This
/// is the stale-entry class of bug: the index tombstone dedup used to `continue`
/// past a tombstone without recording the node as seen, so the iterator reached
/// an older live entry and emitted it — a moved node matched at BOTH cells.
#[tokio::test]
async fn a_moved_geometry_matches_only_at_its_new_position() {
    let (_storage, engine, _tmp) = setup().await;
    insert_shop(&engine, "wanderer", CENTER_LON, CENTER_LAT, "L1").await;

    let here = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 100)"
    );
    assert_eq!(
        names(&engine, &here).await.expect("query failed"),
        vec!["wanderer".to_string()]
    );

    // Move it ~25 km away via SQL UPDATE.
    let moved_lat = CENTER_LAT + 0.225;
    let update = format!(
        "UPDATE '{WS}' SET properties = '{{\"location\": {{\"type\": \"Point\", \
         \"coordinates\": [{CENTER_LON}, {moved_lat}]}}, \"floor\": \"L1\"}}'::jsonb \
         WHERE path = '/wanderer'"
    );
    run(&engine, &update).await;

    assert!(
        names(&engine, &here)
            .await
            .expect("query failed")
            .is_empty(),
        "the old position must no longer match"
    );
    let there = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {moved_lat}), 100)"
    );
    assert_eq!(
        names(&engine, &there).await.expect("query failed"),
        vec!["wanderer".to_string()],
        "the new position must match"
    );
}

/// A deleted node must match nowhere.
#[tokio::test]
async fn a_deleted_geometry_matches_nowhere() {
    let (_storage, engine, _tmp) = setup().await;
    insert_shop(&engine, "doomed", CENTER_LON, CENTER_LAT, "L1").await;

    let sql = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 1000)"
    );
    assert_eq!(names(&engine, &sql).await.expect("query failed").len(), 1);

    run(
        &engine,
        &format!("DELETE FROM '{WS}' WHERE path = '/doomed'"),
    )
    .await;

    assert!(names(&engine, &sql).await.expect("query failed").is_empty());
}

// ── the spatial pseudo-columns, against the real engine ───────────────────

/// A node carrying FOUR stops, the fourth of which is the near one.
///
/// Deliberately index 3: a `__matched_path` implementation that reported the
/// first element, or the pattern it was asked with, would still look plausible on
/// a one- or two-element fixture.
async fn insert_tour(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, name: &str) {
    let stop = |lon: f64, lat: f64| {
        format!("{{\"geo\":{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}}}")
    };
    let properties = format!(
        "{{\"stops\":[{},{},{},{}]}}",
        stop(CENTER_LON, CENTER_LAT + 0.225),
        stop(CENTER_LON, CENTER_LAT + 0.300),
        stop(CENTER_LON, CENTER_LAT + 0.400),
        // ~37 m north of the centre.
        stop(CENTER_LON, CENTER_LAT + 0.000_33),
    );
    let sql = format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
         ('{name}','/{name}','test:Shop', '{properties}'::JSONB)"
    );
    run(engine, &sql).await;
}

/// `__distance` and `__matched_path` are SELECTABLE, and on a wildcard path
/// `__matched_path` names the CONCRETE element that achieved the minimum
/// distance.
///
/// That value is the whole reason the wildcard spelling exists — "which of this
/// node's geometries matched?" — and it was computed and unreachable: injected on
/// the row, undeclared in the catalog, so naming it failed analysis.
#[tokio::test]
async fn the_spatial_columns_name_the_geometry_that_matched() {
    let (_storage, engine, _tmp) = setup().await;
    insert_tour(&engine, "tour").await;

    let sql = format!(
        "SELECT name, __distance, __matched_path FROM '{WS}' \
         WHERE ST_DWITHIN(CAST(properties->>'stops[].geo' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
    );
    let mut stream = engine.execute(&sql).await.expect("query must analyze");
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        rows.push(row.expect("row"));
    }

    assert_eq!(rows.len(), 1, "one node, one row");
    let row = &rows[0];

    let matched = match row.get("__matched_path") {
        Some(PropertyValue::String(path)) => path.clone(),
        other => panic!("__matched_path must be a selectable Text column, got {other:?}"),
    };
    assert_eq!(
        matched, "stops.3.geo",
        "__matched_path must name the CONCRETE element that matched, not the \
         wildcard pattern and not the first element"
    );

    let distance = match row.get("__distance") {
        Some(PropertyValue::Float(d)) => *d,
        other => panic!("__distance must be a selectable Double column, got {other:?}"),
    };
    assert!(
        distance < 100.0,
        "__distance must be the MINIMUM over the matched geometries (~37 m), got {distance} m \
         — the first-found or maximum would be tens of kilometres"
    );
}

/// `SELECT *` must not carry them. They are NULL on every non-spatial access
/// path, so expanding them would add two always-NULL columns to every query in
/// the system.
#[tokio::test]
async fn select_star_does_not_carry_the_spatial_columns() {
    let (_storage, engine, _tmp) = setup().await;
    insert_tour(&engine, "tour").await;

    let sql = format!(
        "SELECT * FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'stops[].geo' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
    );
    let mut stream = engine.execute(&sql).await.expect("query must analyze");
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        rows.push(row.expect("row"));
    }
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].get("__distance").is_none() && rows[0].get("__matched_path").is_none(),
        "`SELECT *` must not expand the spatial pseudo-columns"
    );
    // A sanity anchor: the row IS the wildcard match, so the columns' absence is
    // about expansion, not about the query having matched nothing.
    assert!(rows[0].get("name").is_some());
}

// ── the per-cell budget degrades, it does not fail ────────────────────────

/// Storage whose per-cell spatial scan budget is `budget` entries.
async fn storage_with_cell_budget(budget: usize) -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let config = raisin_rocksdb::RocksDBConfig::development()
        .with_path(temp_dir.path().to_string_lossy().to_string())
        .with_spatial_max_entries_per_cell(budget);
    let storage = raisin_rocksdb::RocksDBStorage::with_config(config).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    (Arc::new(storage), temp_dir)
}

/// THE degradation. When a cell prefix blows the per-cell budget the index
/// cannot answer without answering SHORT — and a short spatial answer is the one
/// outcome this subsystem refuses. It used to fail the query; it must now fall
/// back to a row scan and return the truth.
#[tokio::test]
async fn a_cell_budget_exhaustion_degrades_to_a_row_scan_instead_of_failing() {
    // A budget of one entry: any cell holding two shops is over it.
    let (storage, _tmp) = storage_with_cell_budget(1).await;
    let engine = engine_for(&storage).await;
    seed(&engine).await;

    // 1. The INDEX genuinely refuses this query — otherwise the assertion below
    //    would pass for the boring reason that nothing degraded.
    let precisions = spatial_precisions(&storage, "location");
    let refusal = raisin_storage::SpatialIndexRepository::find_within_radius(
        storage.spatial_index(),
        TENANT,
        REPO,
        BRANCH,
        WS,
        "location",
        CENTER_LON,
        CENTER_LAT,
        50_000.0,
        &raisin_hlc::HLC::now(),
        usize::MAX,
        &precisions,
        &raisin_storage::spatial::SpatialPreFilter::default(),
    )
    .expect_err("a 1-entry budget must be exceeded by three shops");
    assert!(
        refusal.is_spatial_budget_exceeded(),
        "the budget must be a TYPED signal the executor can re-plan against, got: {refusal}"
    );

    // 2. And the query was PLANNED against that index — otherwise it would be
    //    passing for the boring reason that the planner never chose the index at
    //    all, and would keep passing if the degradation were deleted.
    let plan = explain(
        &engine,
        &format!(
            "EXPLAIN SELECT name FROM '{WS}' WHERE              ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY),              ST_POINT({CENTER_LON}, {CENTER_LAT}), 50000)"
        ),
    )
    .await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "this test only exercises the degradation if the index scan was chosen; plan was:\n{plan}"
    );
    assert!(
        plan.contains("degrades to a row scan"),
        "EXPLAIN must show the degradation path, plan was:\n{plan}"
    );

    // 3. The QUERY still returns the truth, via the fallback.
    let sql = format!(
        "SELECT name FROM '{WS}' WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 50000)"
    );
    let rows = names(&engine, &sql)
        .await
        .expect("a budget exhaustion must DEGRADE, not fail the query");
    assert_eq!(
        rows,
        vec!["far".to_string(), "mid".to_string(), "near".to_string()],
        "the degraded scan must return exactly the rows within the radius"
    );
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
        Some(PropertyValue::String(plan)) => plan.clone(),
        other => panic!("unexpected EXPLAIN output: {other:?}"),
    }
}

/// The indexed precisions for `property`, read from the same state record the
/// planner reads.
fn spatial_precisions(storage: &Arc<raisin_rocksdb::RocksDBStorage>, property: &str) -> Vec<usize> {
    let state = raisin_storage::Storage::spatial_state(&**storage)
        .expect("the RocksDB backend reports spatial index state");
    let availability = state.spatial_availability(TENANT, REPO, BRANCH, WS, property);
    assert!(
        availability.is_ready(),
        "the first geometry write should have made the index Ready, got {availability:?}"
    );
    availability.precisions().to_vec()
}

// ── historical reads must be exact ────────────────────────────────────────

/// A spatial read scoped to an explicit revision must NOT resolve against the
/// index, and must still return the truth.
///
/// The index is pruned by a compaction filter that keeps the newest entry per
/// node per cell plus a bounded recent window. That makes HEAD exact and
/// anything behind the window approximate — so a historical spatial query goes
/// to the row scan, whose MVCC history is intact. Both directions are asserted:
/// a HEAD query must keep the index, or this gate has quietly undone the whole
/// spatial performance story.
#[tokio::test]
async fn a_revision_scoped_spatial_query_avoids_the_pruned_index() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let predicate = format!(
        "ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
    );
    // A revision at "now" in milliseconds: everything seeded is visible, so the
    // ROWS are the same and only the ACCESS PATH may differ.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;

    let head_plan = explain(
        &engine,
        &format!("EXPLAIN SELECT name FROM '{WS}' WHERE {predicate}"),
    )
    .await;
    assert!(
        head_plan.contains("SpatialDistanceScan"),
        "a HEAD spatial query must still use the index — the newest entry per node \
         per cell is never pruned, so HEAD is exact. Plan was:\n{head_plan}"
    );

    let historical_plan = explain(
        &engine,
        &format!("EXPLAIN SELECT name FROM '{WS}' WHERE __revision = {now_ms} AND {predicate}"),
    )
    .await;
    assert!(
        !historical_plan.contains("SpatialDistanceScan"),
        "a revision-scoped spatial query must not resolve against the pruned index. \
         Plan was:\n{historical_plan}"
    );

    let head_rows = names(
        &engine,
        &format!("SELECT name FROM '{WS}' WHERE {predicate}"),
    )
    .await
    .expect("head query");
    let historical_rows = names(
        &engine,
        &format!("SELECT name FROM '{WS}' WHERE __revision = {now_ms} AND {predicate}"),
    )
    .await
    .expect("historical query");
    assert_eq!(
        historical_rows, head_rows,
        "the fallback must return the same truth as the index at a revision where \
         nothing has changed"
    );
    assert_eq!(head_rows, vec!["mid".to_string(), "near".to_string()]);
}

// ---------------------------------------------------------------------------
// ST_3DDWITHIN — the ROWS, not the plan
// ---------------------------------------------------------------------------
//
// `ST_3DDWITHIN` now reuses the 2-D `ST_DWITHIN` access path: the cell ring of
// radius `d` is a conservative superset, because horizontal distance is never
// greater than 3D distance. That narrowing is only safe while the predicate
// SURVIVES as a residual filter, so the altitude component is still applied per
// candidate row.
//
// `planner/tests_spatial.rs` asserts the plan keeps the filter. It cannot
// assert that the filter then EXCLUDES anything — which is the half that turns
// "we now use the index" into "we now return the wrong rows". These tests are
// that half.
//
// These run against the REAL index path, not the fallback: `QueryEngine` wires
// `storage.spatial_state()` into the catalog, and a SQL insert of a geometry
// calls `ensure_for_write`, which creates the state record. So the planner sees
// a built index and picks `SpatialDistanceScan` — which
// `st_3ddwithin_uses_the_index_and_still_excludes_by_altitude` asserts via
// EXPLAIN in the same test that checks the rows, so the access path and the
// answer are pinned together rather than in two tests that could drift.

/// Insert a sensor carrying a 3-D geometry: `[lon, lat, altitude]`.
async fn insert_sensor(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    name: &str,
    lon: f64,
    lat: f64,
    altitude: f64,
) {
    let sql = format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
         ('{name}','/{name}','test:Shop', \
          '{{\"location\":{{\"type\":\"Point\",\"coordinates\":[{lon},{lat},{altitude}]}},\
             \"floor\":\"L1\"}}'::JSONB)"
    );
    let mut stream = engine
        .execute(&sql)
        .await
        .unwrap_or_else(|e| panic!("insert failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("insert row error: {e}"));
    }
}

/// Three sensors at the SAME horizontal position, differing only in altitude.
///
/// Horizontally every one of them is a candidate for any radius, so whatever
/// separates them in the result set can only be the altitude component.
async fn seed_altitudes(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    insert_sensor(engine, "ground", CENTER_LON, CENTER_LAT, 0.0).await;
    insert_sensor(engine, "low", CENTER_LON, CENTER_LAT, 100.0).await;
    insert_sensor(engine, "high", CENTER_LON, CENTER_LAT, 5_000.0).await;
}

/// THE regression this guards. All three sensors are horizontally identical, so
/// a 3D radius of 200 m must keep exactly the two within 200 m vertically.
///
/// If the predicate were stripped once the index was chosen, every row inside
/// the horizontal ring would come back and `high` — 5 km straight up — would
/// appear. That is the silent-wrong-answer this pairing exists to prevent.
#[tokio::test]
async fn st_3ddwithin_excludes_rows_that_are_only_far_in_altitude() {
    let (_storage, engine, _tmp) = setup().await;
    seed_altitudes(&engine).await;

    let rows = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_3DDWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_FORCE3D(ST_POINT({CENTER_LON}, {CENTER_LAT}), 0), 200)"
        ),
    )
    .await
    .expect("3D query must not error");

    assert_eq!(
        rows,
        vec!["ground".to_string(), "low".to_string()],
        "altitude must still filter after the 2-D index narrows the candidates \
         — 'high' is 5 km up and horizontally identical, so its presence would \
         mean the residual filter was dropped",
    );
}

/// The 2-D and 3-D predicates must disagree exactly where altitude says so.
///
/// Same centre, same radius, same rows on the ground: `ST_DWITHIN` takes all
/// three, `ST_3DDWITHIN` takes two. If they ever agreed here, the 3D predicate
/// would be doing nothing.
#[tokio::test]
async fn the_3d_predicate_is_strictly_stronger_than_the_2d_one() {
    let (_storage, engine, _tmp) = setup().await;
    seed_altitudes(&engine).await;

    let flat = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_DWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_POINT({CENTER_LON}, {CENTER_LAT}), 200)"
        ),
    )
    .await
    .expect("2D query must not error");

    let solid = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_3DDWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_FORCE3D(ST_POINT({CENTER_LON}, {CENTER_LAT}), 0), 200)"
        ),
    )
    .await
    .expect("3D query must not error");

    assert_eq!(
        flat,
        vec!["ground".to_string(), "high".to_string(), "low".to_string()],
        "horizontally all three are within 200 m",
    );
    assert!(
        solid.len() < flat.len(),
        "the 3D predicate must be strictly stronger than the 2-D one, got \
         2D={flat:?} 3D={solid:?}",
    );
}

/// Raising the query altitude changes WHICH rows match — the altitude of the
/// centre is genuinely read, not ignored.
///
/// A centre at 5000 m must select `high` and reject the two near the ground:
/// the exact inverse of the ground-level query over identical data.
#[tokio::test]
async fn the_query_altitude_selects_a_different_row_set() {
    let (_storage, engine, _tmp) = setup().await;
    seed_altitudes(&engine).await;

    let rows = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_3DDWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_FORCE3D(ST_POINT({CENTER_LON}, {CENTER_LAT}), 5000), 200)"
        ),
    )
    .await
    .expect("3D query must not error");

    assert_eq!(
        rows,
        vec!["high".to_string()],
        "a centre 5 km up must match only the sensor at that altitude — if the \
         centre's Z were dropped this would return the ground-level rows",
    );
}

/// A 2-D geometry has no altitude, and must not be silently treated as z = 0.
///
/// This pins the mixed-dimension case: the rows are the ordinary 2-D shops, and
/// the query is a 3D one. Whatever the answer is, it must not error and must not
/// return a row the plain 2-D predicate would exclude.
#[tokio::test]
async fn a_3d_predicate_over_2d_data_stays_within_the_2d_answer() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let flat = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_DWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
        ),
    )
    .await
    .expect("2D query must not error");

    let solid = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_3DDWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_FORCE3D(ST_POINT({CENTER_LON}, {CENTER_LAT}), 0), 500)"
        ),
    )
    .await
    .expect("3D query over 2-D data must not error");

    for name in &solid {
        assert!(
            flat.contains(name),
            "the 3D predicate returned {name}, which the 2-D predicate excludes \
             — the horizontal ring is supposed to be a SUPERSET of the answer, \
             so this direction can never be wider: 2D={flat:?} 3D={solid:?}",
        );
    }
}

/// THE gap this closes: the index path and the answer, pinned in ONE test.
///
/// Everything else about `ST_3DDWITHIN` is proven in two halves that can drift.
/// `planner/tests_spatial.rs` says the plan is a `SpatialDistanceScan` carrying
/// a residual filter; the row tests above say altitude excludes the right
/// sensors. Neither says those two things happen *at the same time* — and the
/// dangerous state is exactly the one where the index is chosen and the filter
/// is not applied, because that returns a superset silently.
///
/// So: assert EXPLAIN picked the index, then assert the rows, over the same
/// engine and the same data.
#[tokio::test]
async fn st_3ddwithin_uses_the_index_and_still_excludes_by_altitude() {
    let (storage, engine, _tmp) = setup().await;
    seed_altitudes(&engine).await;

    // Precondition: the geometry write built the index state the planner reads.
    // Without this the assertions below would pass for the wrong reason — a
    // fallback scan also returns the right rows.
    assert!(
        !spatial_precisions(&storage, "location").is_empty(),
        "the SQL insert must have created the spatial state record, otherwise \
         this test silently degrades into a fallback-scan test",
    );

    let predicate = format!(
        "ST_3DDWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_FORCE3D(ST_POINT({CENTER_LON}, {CENTER_LAT}), 0), 200)"
    );

    let plan = explain(
        &engine,
        &format!("EXPLAIN SELECT name FROM '{WS}' WHERE {predicate}"),
    )
    .await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "ST_3DDWITHIN must narrow through the 2-D index — the horizontal ring is \
         a conservative superset, so there is no reason to scan. Plan was:\n{plan}"
    );

    let rows = names(
        &engine,
        &format!("SELECT name FROM '{WS}' WHERE {predicate}"),
    )
    .await
    .expect("3D query must not error");

    assert_eq!(
        rows,
        vec!["ground".to_string(), "low".to_string()],
        "the index was used AND altitude still excluded the sensor 5 km up. \
         Returning 'high' here would be the silent superset: index chosen, \
         residual filter dropped. Plan was:\n{plan}"
    );
}

/// The index path and the fallback path must agree on the same data.
///
/// A differential check: the same 3D question asked with the index available
/// and asked at a historical revision (which the planner routes off the index)
/// must produce identical rows. This is the shape that catches an index that is
/// subtly *incomplete* rather than absent — the fallback is the reference
/// answer.
#[tokio::test]
async fn the_3d_index_path_agrees_with_the_fallback_path() {
    let (_storage, engine, _tmp) = setup().await;
    seed_altitudes(&engine).await;

    let predicate = format!(
        "ST_3DDWITHIN(CAST(properties->>'location' AS GEOMETRY), \
         ST_FORCE3D(ST_POINT({CENTER_LON}, {CENTER_LAT}), 0), 200)"
    );
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;

    let indexed = names(
        &engine,
        &format!("SELECT name FROM '{WS}' WHERE {predicate}"),
    )
    .await
    .expect("indexed query");

    // A revision-scoped read is deliberately routed off the pruned index.
    let scanned = names(
        &engine,
        &format!("SELECT name FROM '{WS}' WHERE __revision = {now_ms} AND {predicate}"),
    )
    .await
    .expect("fallback query");

    assert_eq!(
        indexed, scanned,
        "the index path and the row scan must agree — everything was seeded \
         before this revision, so only the ACCESS PATH differs",
    );
    assert_eq!(indexed, vec!["ground".to_string(), "low".to_string()]);
}

/// The geometry-argument spelling: both forms work, and must AGREE.
///
/// The module header above says only `CAST(properties->>'…' AS GEOMETRY)`
/// survives analysis and that a bare column fails. That is no longer true — the
/// bare `location` spelling, which the RaisinDB SQL skill and the website both
/// use throughout, analyses fine.
///
/// What matters is that it returns the SAME rows. A spelling that analysed but
/// resolved to NULL would return zero rows silently, which is the worst outcome
/// and precisely what this module exists to catch.
#[tokio::test]
async fn both_geometry_spellings_return_the_same_rows() {
    let (_storage, engine, _tmp) = setup().await;
    seed(&engine).await;

    let cast_form = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_DWITHIN(\
               CAST(properties->>'location' AS GEOMETRY), \
               ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
        ),
    )
    .await
    .expect("the CAST spelling must analyse");

    let bare_form = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' \
             WHERE ST_DWITHIN(location, ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
        ),
    )
    .await
    .expect("the bare-column spelling must analyse — it is what the docs use");

    assert_eq!(
        bare_form, cast_form,
        "the two documented spellings must return identical rows; a bare column \
         that resolved to NULL would return zero rows with no error",
    );
    assert_eq!(cast_form, vec!["mid".to_string(), "near".to_string()]);

    // The third spelling: the JSON extraction with no cast.
    let uncast = names(
        &engine,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_DWITHIN(\
               properties->>'location', ST_POINT({CENTER_LON}, {CENTER_LAT}), 500)"
        ),
    )
    .await
    .expect("the uncast spelling analyses too");

    assert_eq!(
        uncast, cast_form,
        "all three spellings must agree; the module header used to claim only \
         the CAST form analysed, and that is no longer true",
    );
}
