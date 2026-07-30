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

    (storage, engine, tmp)
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
