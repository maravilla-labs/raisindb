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
//! These queries write the geometry argument as
//! `CAST(properties->>'location' AS GEOMETRY)`, because that is the only one of
//! the three spellings that survives ANALYSIS today:
//!
//! * `ST_DWITHIN(properties->>'location', ...)` fails with
//!   `Function not found: ST_DWITHIN(TEXT?, GEOMETRY, DOUBLE)` — `Text -> Geometry`
//!   is in the explicit-cast table but not in the implicit coercion ladder, and no
//!   registered `ST_*` signature accepts `Text`.
//! * `ST_DWITHIN(location, ...)` — the spelling the website documents — fails with
//!   `Column not found: <ws>.location`, because a geometry stored in `properties`
//!   is not a declared column of the workspace schema.
//!
//! Both are signature/analysis gaps, not planner gaps: the planner's
//! `extract_geometry_source` handles all three forms identically. They are worth
//! fixing, because a documented spelling that does not analyse is its own kind of
//! silent failure.

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
