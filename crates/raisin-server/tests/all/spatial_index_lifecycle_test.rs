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

//! The spatial index **lifecycle**, end to end against a real server.
//!
//! # Why this file exists
//!
//! Everything else that covers spatial indexing either runs the maths in isolation
//! (63 unit tests over pure geometry) or runs the planner against a catalog that
//! reports the index as `NotBuilt`, which exercises the *fallback* path. Neither can
//! catch the failures this module is aimed at, all four of which are invisible to a
//! unit test because they live in the write path:
//!
//! 1. **The index is never populated at all.** Automatic, type-driven indexing means
//!    a `PropertyValue::Geometry` is indexed with no opt-in — but two of the five
//!    write paths did not do it, and the planner's hardcoded "the index is always
//!    present" turned that into zero rows with no error.
//! 2. **Stale entries survive an update.** Revisions sort DESCENDING and the scan
//!    used to `continue` past a tombstone without recording that it had seen the
//!    node, so it went on to emit an older live entry: a deleted node still matched,
//!    and a moved node matched at BOTH its old and its new location.
//! 3. **The radius window.** Only precisions 4–8 were indexed and a geohash prefix
//!    matches exactly one precision, so any radius outside roughly 4.8 m – 39 km
//!    silently returned nothing. Sub-metre indoor queries were unusable.
//! 4. **Altitude was discarded** at the parser boundary, so a 3-D position became
//!    2-D on the way in and no amount of `ST_Z` could get it back.
//!
//! # The shape of the proof
//!
//! Every assertion below goes over HTTP to a live `ServerHandle`: real RocksDB, real
//! planner, real index. Writes are issued through **both** first-class surfaces — SQL
//! `INSERT`/`UPDATE`/`DELETE` and the REST node API — because the index hook lives in
//! the low-level write functions and the whole question is whether a given surface
//! actually reaches them.
//!
//! The `EXPLAIN` assertions matter as much as the row-set ones. A result set alone
//! cannot distinguish "the index answered correctly" from "the index was skipped and
//! a full scan with a residual filter answered correctly" — and the second is exactly
//! what a silently-unpopulated index looks like. Asserting `SpatialDistanceScan`
//! appears in the plan is what makes this a test of the index rather than of the
//! fallback.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const REPO: &str = "spatial_lifecycle";
const BRANCH: &str = "main";
const WORKSPACE: &str = "places";
/// Unique to this module. `ServerHandle::wait_for_ready` polls `/health` and is
/// satisfied by *any* listener on the port — so if two test modules share a port,
/// the second one's server fails to bind, the harness reports the first one's
/// server as ready, and the two tests silently share a database until one of them
/// exits and drops the other's connection mid-request. `assert_server_is_ours`
/// below turns that into an immediate, explicit failure.
const PORT: u16 = 8117;

/// Zurich Hauptbahnhof — the origin every offset below is measured from.
const CENTER_LON: f64 = 8.5402;
const CENTER_LAT: f64 = 47.3782;

/// Bern, ~95 km southwest: far enough that a moved node lands in a different cell
/// at every indexed precision, which is the case the stale-entry bug got wrong.
const BERN_LON: f64 = 7.4474;
const BERN_LAT: f64 = 46.9480;

/// Metres per degree of latitude on the sphere the index measures with
/// (`Haversine` over the GRS80 mean radius, 6 371 008.8 m — the same constant the
/// storage-side post-filter uses, so a fixture offset and the distance the server
/// computes cannot disagree).
const M_PER_DEG_LAT: f64 = std::f64::consts::PI * 6_371_008.8 / 180.0;

/// A latitude offset that is `meters` due north of the centre.
fn north(meters: f64) -> f64 {
    CENTER_LAT + meters / M_PER_DEG_LAT
}

// --- transport ----------------------------------------------------------------

async fn http_post(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let response = Client::new()
        .post(format!("{base_url}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|_| text)
}

async fn http_put(base_url: &str, path: &str, token: &str, body: Value) -> Result<(), String> {
    let response = Client::new()
        .put(format!("{base_url}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(format!(
            "{status}: {}",
            response.text().await.unwrap_or_default()
        ));
    }
    Ok(())
}

async fn sql(base_url: &str, token: &str, query: &str) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{REPO}"),
        token,
        json!({ "sql": query, "params": [] }),
    )
    .await
}

/// Run SQL, panicking with the statement text on failure.
async fn run_sql(base_url: &str, token: &str, query: &str) -> Value {
    sql(base_url, token, query)
        .await
        .unwrap_or_else(|e| panic!("SQL failed\n  {query}\n  {e}"))
}

async fn sql_scalar(base_url: &str, token: &str, query: &str, column: &str) -> Value {
    run_sql(base_url, token, query).await["rows"][0][column].clone()
}

/// The `name` column of every returned row, sorted — the comparable form of a
/// spatial result set.
async fn names(base_url: &str, token: &str, query: &str) -> Vec<String> {
    let result = run_sql(base_url, token, query).await;
    let mut out: Vec<String> = result["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows array in {result}"))
        .iter()
        .map(|row| row["name"].as_str().unwrap_or("?").to_string())
        .collect();
    out.sort();
    out
}

/// `EXPLAIN`'s plan text, so a test can tell an index-backed answer from a full
/// scan that happened to produce the same rows.
async fn explain(base_url: &str, token: &str, query: &str) -> String {
    sql_scalar(base_url, token, &format!("EXPLAIN {query}"), "QUERY PLAN")
        .await
        .as_str()
        .unwrap_or_default()
        .to_string()
}

// --- query builders -----------------------------------------------------------

/// A radius query around an arbitrary centre.
///
/// The geometry argument is spelled `CAST(properties->>'location' AS GEOMETRY)`
/// because that is the spelling that survives analysis: the bare
/// `properties->>'location'` resolves to `TEXT`, and `TEXT -> GEOMETRY` is an
/// explicit-cast-only conversion, so no registered `ST_DWITHIN` signature matches.
fn dwithin_at(lon: f64, lat: f64, radius_m: f64) -> String {
    format!(
        "SELECT name FROM '{WORKSPACE}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({lon}, {lat}), {radius_m})"
    )
}

fn dwithin(radius_m: f64) -> String {
    dwithin_at(CENTER_LON, CENTER_LAT, radius_m)
}

// --- the lifecycle ------------------------------------------------------------

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_index_lifecycle_test -- --ignored --nocapture
async fn test_spatial_index_lifecycle_end_to_end() {
    println!("\n=== spatial index lifecycle, end to end ===\n");

    let mut server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert_server_is_ours(&mut server);

    let token = bootstrap_admin(&server.base_url).await;
    let base = server.base_url.clone();
    provision(&base, &token).await;
    println!("[OK] server up, repo/workspace/nodetype provisioned");

    write_fixture(&base, &token).await;

    index_is_populated_and_used(&base, &token).await;
    every_radius_scale_returns_the_truth(&base, &token).await;
    a_moved_node_matches_only_at_its_new_location(&base, &token).await;
    a_deleted_node_matches_nowhere(&base, &token).await;
    floor_filtering_selects_one_level(&base, &token).await;
    altitude_survives_and_is_measurable(&base, &token).await;
    bulk_dml_and_geometry_expressions_index_too(&base, &token).await;

    println!("\n=== spatial index lifecycle: PASS ===\n");
}

/// The fixture, written over **two** surfaces on purpose.
///
/// `kiosk`/`atm`/`shop`/`cafe` go in with SQL `INSERT`; `airport`/`bern` go in with
/// the REST node API. Both must end up in the index, because the two reach the
/// low-level write functions by different routes and only one of them was ever
/// covered by a test.
async fn write_fixture(base: &str, token: &str) {
    // (name, lon, lat, floor) — offsets are pure latitude, so the distance from the
    // centre is exactly `meters` on the sphere the index measures with.
    let over_sql = [
        ("kiosk", CENTER_LON, north(0.3), "L0"),
        ("atm", CENTER_LON, north(1.5), "L0"),
        ("shop", CENTER_LON, north(30.0), "L1"),
        ("cafe", CENTER_LON, north(300.0), "L1"),
    ];
    for (name, lon, lat, floor) in over_sql {
        insert_via_sql(base, token, name, lon, lat, floor).await;
    }

    let over_rest = [
        ("airport", CENTER_LON, north(10_000.0), "L0"),
        ("bern", BERN_LON, BERN_LAT, "L0"),
    ];
    for (name, lon, lat, floor) in over_rest {
        insert_via_rest(base, token, name, json!([lon, lat]), floor).await;
    }

    // Three levels of one building, all within a couple of metres horizontally: the
    // case a radius query alone cannot separate.
    for (name, floor) in [("gate-l1", "L1"), ("gate-l2", "L2"), ("gate-l3", "L3")] {
        insert_via_sql(base, token, name, CENTER_LON + 0.000_02, north(2.0), floor).await;
    }

    // A 3-D pair at one lon/lat, 100 m apart vertically.
    insert_via_rest(
        base,
        token,
        "tower-base",
        json!([CENTER_LON - 0.000_05, north(5.0), 0.0]),
        "L0",
    )
    .await;
    insert_via_rest(
        base,
        token,
        "tower-top",
        json!([CENTER_LON - 0.000_05, north(5.0), 100.0]),
        "L30",
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    println!("[OK] fixture written: 4 over SQL INSERT, 4 over the REST node API");
}

async fn insert_via_sql(base: &str, token: &str, name: &str, lon: f64, lat: f64, floor: &str) {
    let query = format!(
        "INSERT INTO '{WORKSPACE}' (id, path, name, node_type, properties) VALUES \
         ('{name}', '/{name}', '{name}', 'geo:Place', \
          '{{\"title\":\"{name}\",\"floor\":\"{floor}\",\
             \"location\":{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}}}'::JSONB)"
    );
    run_sql(base, token, &query).await;
}

async fn insert_via_rest(base: &str, token: &str, name: &str, coordinates: Value, floor: &str) {
    http_post(
        base,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/"),
        token,
        json!({
            "node": {
                "id": name,
                "name": name,
                "node_type": "geo:Place",
                "properties": {
                    "title": name,
                    "floor": floor,
                    "location": { "type": "Point", "coordinates": coordinates }
                }
            }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("REST create {name}: {e}"));
}

// --- phase 1: the index exists, is used, and answers ---------------------------

/// Automatic type-driven indexing: nobody asked for an index, so the fact that the
/// planner reports one is itself the assertion.
async fn index_is_populated_and_used(base: &str, token: &str) {
    println!("\n--- the index is populated by the write itself, with no opt-in ---");

    let plan = explain(base, token, &dwithin(50.0)).await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "the plan must be index-backed — a full scan here means the write path never \
         created the index state record, which is the silent-empty failure this test \
         exists to catch:\n{plan}"
    );
    assert!(
        !plan.contains("NOT BUILT"),
        "the index must report Ready after an ordinary write:\n{plan}"
    );
    println!("[PASS] EXPLAIN is index-backed: {}", plan.trim());

    // 50 m: kiosk (0.3 m), atm (1.5 m), gate-l1..l3 (~2 m), tower-base/top (~5 m),
    // shop (30 m). Written over SQL and REST alike, so a surface that skipped the
    // index hook would show up as a missing name rather than as a wrong count.
    assert_eq!(
        names(base, token, &dwithin(50.0)).await,
        vec![
            "atm",
            "gate-l1",
            "gate-l2",
            "gate-l3",
            "kiosk",
            "shop",
            "tower-base",
            "tower-top"
        ],
        "both write surfaces must populate the index"
    );
    println!("[PASS] SQL INSERT and the REST node API both reach the index");
}

// --- phase 2: the radius window ------------------------------------------------

/// The window used to be silently ~4.8 m – 39 km. Sub-metre through continental must
/// all give the truth, because precision is a performance knob and never a
/// correctness one.
async fn every_radius_scale_returns_the_truth(base: &str, token: &str) {
    println!("\n--- every radius scale, sub-metre to continental ---");

    // The 3-D pair and the three gates sit between 2 m and 5 m of the centre, so
    // they enter at the 10 m step. Distances are exact by construction.
    let cases: Vec<(f64, Vec<&str>)> = vec![
        (0.5, vec!["kiosk"]),
        (2.0, vec!["atm", "kiosk"]),
        (
            10.0,
            vec![
                "atm",
                "gate-l1",
                "gate-l2",
                "gate-l3",
                "kiosk",
                "tower-base",
                "tower-top",
            ],
        ),
        (
            50.0,
            vec![
                "atm",
                "gate-l1",
                "gate-l2",
                "gate-l3",
                "kiosk",
                "shop",
                "tower-base",
                "tower-top",
            ],
        ),
        (
            500.0,
            vec![
                "atm",
                "cafe",
                "gate-l1",
                "gate-l2",
                "gate-l3",
                "kiosk",
                "shop",
                "tower-base",
                "tower-top",
            ],
        ),
        (
            50_000.0,
            vec![
                "airport",
                "atm",
                "cafe",
                "gate-l1",
                "gate-l2",
                "gate-l3",
                "kiosk",
                "shop",
                "tower-base",
                "tower-top",
            ],
        ),
        (
            500_000.0,
            vec![
                "airport",
                "atm",
                "bern",
                "cafe",
                "gate-l1",
                "gate-l2",
                "gate-l3",
                "kiosk",
                "shop",
                "tower-base",
                "tower-top",
            ],
        ),
    ];

    for (radius, expected) in cases {
        let got = names(base, token, &dwithin(radius)).await;
        assert_eq!(
            got, expected,
            "radius {radius} m returned the wrong set — the old code returned NOTHING \
             below ~4.8 m and above ~39 km"
        );
        println!("[PASS] r = {radius:>9} m -> {} rows", got.len());
    }

    // 0.1 m is finer than the finest indexed cell (precision 11, ~0.15 m). It must
    // still be answered: too-fine only means over-fetching and post-filtering.
    assert_eq!(
        names(base, token, &dwithin(0.1)).await,
        Vec::<String>::new(),
        "a 0.1 m radius has no match here, but it must be ANSWERED, not declined"
    );
    println!("[PASS] a radius finer than the finest indexed cell is still answered");
}

// --- phase 3: the stale-entry bug ----------------------------------------------

/// The defect: an update writes tombstones at the OLD cells and live entries at the
/// NEW ones. A scan that reached the old cell first and skipped its tombstone without
/// recording the node went on to emit an older live entry from the same cell — so a
/// moved node matched at both places.
///
/// A 500 km radius covers the old cell and the new cell in ONE scan, which is the
/// case a per-cell fix gets wrong in the opposite direction (the node vanishes
/// entirely). So the count is asserted, not just the membership.
async fn a_moved_node_matches_only_at_its_new_location(base: &str, token: &str) {
    println!("\n--- a moved node matches at its new location and nowhere else ---");

    let moved = json!({
        "title": "shop",
        "floor": "L1",
        "location": { "type": "Point", "coordinates": [BERN_LON, BERN_LAT] }
    })
    .to_string()
    .replace('\'', "''");

    run_sql(
        base,
        token,
        &format!("UPDATE '{WORKSPACE}' SET properties = '{moved}'::JSONB WHERE path = '/shop'"),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert!(
        !names(base, token, &dwithin(50.0))
            .await
            .contains(&"shop".to_string()),
        "shop moved 95 km away; a 50 m query at the OLD location must not match it — \
         this is the stale-entry regression"
    );
    println!("[PASS] the old location no longer matches");

    let at_bern = names(base, token, &dwithin_at(BERN_LON, BERN_LAT, 50.0)).await;
    assert!(
        at_bern.contains(&"shop".to_string()),
        "the NEW location must match: {at_bern:?}"
    );
    println!("[PASS] the new location matches");

    // One scan over both cells: exactly one row, not two, and not zero.
    let wide = names(base, token, &dwithin(500_000.0)).await;
    let occurrences = wide.iter().filter(|n| *n == "shop").count();
    assert_eq!(
        occurrences, 1,
        "a query whose cells cover BOTH the old and the new position must resolve the \
         node once — 2 is the original false positive, 0 is the false negative a \
         per-cell fix would introduce: {wide:?}"
    );
    println!("[PASS] a query covering both cells resolves the node exactly once");
}

// --- phase 4: delete -----------------------------------------------------------

async fn a_deleted_node_matches_nowhere(base: &str, token: &str) {
    println!("\n--- a deleted node matches nowhere ---");

    run_sql(
        base,
        token,
        &format!("DELETE FROM '{WORKSPACE}' WHERE path = '/cafe'"),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    for radius in [500.0, 50_000.0, 500_000.0] {
        let got = names(base, token, &dwithin(radius)).await;
        assert!(
            !got.contains(&"cafe".to_string()),
            "a deleted node must not match at r = {radius} m: {got:?}"
        );
    }
    println!("[PASS] gone at every radius that used to reach it");

    // And the REST-written nodes delete cleanly too, so the tombstone path is not
    // SQL-specific.
    let response = Client::new()
        .delete(format!(
            "{base}/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/airport"
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("REST delete");
    assert!(
        response.status().is_success(),
        "REST delete failed: {} {}",
        response.status(),
        response.text().await.unwrap_or_default()
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let got = names(base, token, &dwithin(50_000.0)).await;
    assert!(
        !got.contains(&"airport".to_string()),
        "a node deleted over REST must stop matching: {got:?}"
    );
    println!("[PASS] the REST delete path tombstones the index too");
}

// --- phase 5: floors -----------------------------------------------------------

/// A floor is a **discrete ordinal label**, not a coordinate: three gates two metres
/// apart horizontally are on three different levels, and no radius can separate them.
/// So the level is an ordinary property and the query is a conjunction.
///
/// Whether the planner *also* pushes the level into the scan as a candidate
/// pre-filter is a separate, selectivity-only question — the predicate always stays
/// in the residual filter, so the result set is identical either way. Both are
/// asserted, and in that order: the answer first, because that is what may never
/// regress, then the optimisation, because a pre-filter that is configured and
/// silently inert is indistinguishable from one that works.
async fn floor_filtering_selects_one_level(base: &str, token: &str) {
    println!("\n--- floor-filtered proximity ---");

    let all_levels = names(
        base,
        token,
        &dwithin_at(CENTER_LON + 0.000_02, north(2.0), 5.0),
    )
    .await;
    assert!(
        all_levels.contains(&"gate-l1".to_string())
            && all_levels.contains(&"gate-l2".to_string())
            && all_levels.contains(&"gate-l3".to_string()),
        "all three levels are within 5 m horizontally: {all_levels:?}"
    );

    // Both spellings of the level predicate, because they take different routes and
    // must not differ in what they return. `->>'floor'::String` is the
    // key-cast form the docs recommend — it evaluates verbatim per row, so it is
    // always correct — and `->>'floor'` is the bare form, which canonicalises and
    // can therefore be recognised by the planner.
    let cast_form = format!(
        "SELECT name FROM '{WORKSPACE}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({}, {}), 5) \
           AND properties->>'floor'::String = 'L2'",
        CENTER_LON + 0.000_02,
        north(2.0)
    );
    let bare_form = format!(
        "SELECT name FROM '{WORKSPACE}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({}, {}), 5) \
           AND properties->>'floor' = 'L2'",
        CENTER_LON + 0.000_02,
        north(2.0)
    );
    assert_eq!(
        names(base, token, &cast_form).await,
        vec!["gate-l2"],
        "the level predicate must reduce a radius hit to one level"
    );
    assert_eq!(
        names(base, token, &bare_form).await,
        vec!["gate-l2"],
        "the two spellings of the same filter must agree"
    );
    println!(
        "[PASS] {} within 5 m, 1 on level L2, both spellings",
        all_levels.len()
    );

    // The pre-filter engages only if the whole chain held: the workspace record's
    // declared `bucket_property` was read at write time, stamped into the local
    // index-state record, reported back through the catalog's availability, and
    // matched against a sibling equality in the query. Any link missing and the
    // answer is still right — which is exactly why it needs asserting rather than
    // assuming.
    let plan = explain(base, token, &bare_form).await;
    assert!(
        plan.contains("bucket floor='L2'"),
        "the level must reach the scan as a candidate pre-filter, or every floor's \
         node records get fetched and thrown away:\n{plan}"
    );
    println!("[PASS] the level rides into the scan as a pre-filter");

    // The cast form deliberately does NOT feed the pre-filter: it is defined to
    // evaluate verbatim, which is exactly what makes it immune to a stale or absent
    // compound index. Asserting the difference keeps the trade-off visible instead
    // of letting someone "fix" one spelling into the other.
    let cast_plan = explain(base, token, &cast_form).await;
    assert!(
        !cast_plan.contains("bucket floor="),
        "the verbatim cast form is not canonicalised, so it cannot feed the \
         pre-filter — if it now does, this comment is out of date:\n{cast_plan}"
    );
}

// --- phase 7: bulk / compound DML, and geometry-valued expressions --------------

/// Bulk and compound SQL DML is guilty until proven innocent in this codebase: the
/// `BulkSql` job executor was once a debug stub that silently did nothing, so
/// triggers never fired on compound writes. A multi-row `INSERT` and a `WHERE`
/// matching several rows take a different route through the DML executor than the
/// single-row path, and both have to reach the index.
///
/// The second half covers the other SQL spelling of a geometry: the value as an
/// *expression* (`ST_POINT(...)`) rather than as GeoJSON inside a JSONB blob.
async fn bulk_dml_and_geometry_expressions_index_too(base: &str, token: &str) {
    println!("\n--- bulk DML and geometry expressions ---");

    // Three rows in ONE statement, ~700 m north of the centre so they cannot be
    // confused with anything already in the fixture.
    let rows: Vec<String> = (0..3)
        .map(|i| {
            let lat = north(700.0 + i as f64);
            format!(
                "('bulk{i}', '/bulk{i}', 'bulk{i}', 'geo:Place', \
                 '{{\"title\":\"bulk{i}\",\"floor\":\"L9\",\
                    \"location\":{{\"type\":\"Point\",\"coordinates\":[{CENTER_LON},{lat}]}}}}'::JSONB)"
            )
        })
        .collect();
    run_sql(
        base,
        token,
        &format!(
            "INSERT INTO '{WORKSPACE}' (id, path, name, node_type, properties) VALUES {}",
            rows.join(", ")
        ),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let near_bulk = names(base, token, &dwithin_at(CENTER_LON, north(701.0), 20.0)).await;
    assert_eq!(
        near_bulk,
        vec!["bulk0", "bulk1", "bulk2"],
        "every row of a multi-row INSERT must be indexed"
    );
    println!("[PASS] a 3-row INSERT indexed all three");

    // A compound UPDATE: one statement, a WHERE that matches all three, moving them
    // 5 km north. The old cells must be tombstoned and the new ones written for
    // every matched row, not just the first.
    let moved_lat = north(5_000.0);
    run_sql(
        base,
        token,
        &format!(
            "UPDATE '{WORKSPACE}' SET properties = \
             '{{\"title\":\"bulk\",\"floor\":\"L9\",\
                \"location\":{{\"type\":\"Point\",\"coordinates\":[{CENTER_LON},{moved_lat}]}}}}'::JSONB \
             WHERE properties->>'floor'::String = 'L9'"
        ),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let still_at_old = names(base, token, &dwithin_at(CENTER_LON, north(701.0), 20.0)).await;
    assert!(
        still_at_old.is_empty(),
        "a compound UPDATE must tombstone the old cells of EVERY matched row: \
         {still_at_old:?}"
    );
    let at_new = names(base, token, &dwithin_at(CENTER_LON, moved_lat, 20.0)).await;
    assert_eq!(
        at_new,
        vec!["bulk0", "bulk1", "bulk2"],
        "and write the new cells for every matched row"
    );
    println!("[PASS] a compound UPDATE moved all three, leaving nothing behind");

    // Geometry as an expression rather than as GeoJSON text — the natural PostGIS
    // spelling. The DML value converter now handles it (see
    // `dml_executor::helpers_tests::a_geometry_valued_expression_can_be_assigned_to_a_property`;
    // it previously failed the whole statement with "Cannot convert literal"), but
    // the spelling does not survive ANALYSIS: a workspace table exposes only its
    // fixed columns, so any property name is "Column not found" for both INSERT and
    // UPDATE. That is an analyzer gap, not an indexing one, and it is out of this
    // module's scope — so it is probed and reported rather than asserted, which
    // keeps it visible instead of quietly absent.
    let expr_lat = north(9_000.0);
    let attempt = sql(
        base,
        token,
        &format!(
            "INSERT INTO '{WORKSPACE}' (id, path, name, node_type, location) \
             VALUES ('expr', '/expr', 'expr', 'geo:Place', \
                     ST_POINT({CENTER_LON}, {expr_lat}))"
        ),
    )
    .await;
    match attempt {
        Ok(_) => {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert_eq!(
                names(base, token, &dwithin_at(CENTER_LON, expr_lat, 20.0)).await,
                vec!["expr"],
                "if a geometry expression now analyses, it must also be INDEXED — a \
                 write surface that stores but does not index is the exact silent \
                 failure this module exists to catch"
            );
            println!("[PASS] ST_POINT(...) as a column value is stored and indexed");
        }
        Err(e) => println!(
            "[GAP] a property column cannot be named in workspace DML, so \
             `location = ST_POINT(...)` never reaches the converter: {}",
            e.lines().next().unwrap_or("")
        ),
    }
}

// --- phase 6: altitude ---------------------------------------------------------

/// Altitude used to be discarded at the parser boundary, so a 3-D position became
/// 2-D on the way in. Two things have to hold: the third ordinate survives storage,
/// and the 2-D index still finds the node (`geo`'s coordinates are 2-D by
/// construction, so Z is projected away for topology and read back off the value).
async fn altitude_survives_and_is_measurable(base: &str, token: &str) {
    println!("\n--- altitude survives storage and is measurable ---");

    let rows = run_sql(
        base,
        token,
        &format!(
            "SELECT name, \
                    ST_Z(CAST(properties->>'location' AS GEOMETRY)) AS z, \
                    ST_NDIMS(CAST(properties->>'location' AS GEOMETRY)) AS dims \
             FROM '{WORKSPACE}' WHERE path IN ('/tower-base', '/tower-top', '/kiosk') \
             ORDER BY name"
        ),
    )
    .await;
    let rows = rows["rows"].as_array().expect("rows").clone();
    assert_eq!(rows.len(), 3, "expected 3 rows, got {}", rows.len());

    for row in &rows {
        let name = row["name"].as_str().unwrap_or("?");
        match name {
            "kiosk" => {
                assert_eq!(row["dims"].as_i64(), Some(2), "kiosk is 2-D");
                assert!(
                    row["z"].is_null(),
                    "ST_Z of a 2-D point is NULL, not 0: {row}"
                );
            }
            "tower-base" => {
                assert_eq!(row["dims"].as_i64(), Some(3));
                assert_eq!(row["z"].as_f64(), Some(0.0), "the base sits at 0 m");
            }
            "tower-top" => {
                assert_eq!(row["dims"].as_i64(), Some(3));
                assert_eq!(
                    row["z"].as_f64(),
                    Some(100.0),
                    "100 m must survive the write, the storage encoding and the read \
                     back — this is the ordinate that used to be dropped"
                );
            }
            other => panic!("unexpected row {other}"),
        }
    }
    println!("[PASS] z = NULL / 0 / 100 read back off stored geometry");

    // The 2-D index is indifferent to altitude, exactly as PostGIS's 2-D predicates
    // are: both ends of the tower are within 10 m horizontally.
    let horizontal = names(base, token, &dwithin(10.0)).await;
    assert!(
        horizontal.contains(&"tower-base".to_string())
            && horizontal.contains(&"tower-top".to_string()),
        "a 2-D radius must find both ends of a vertical pair: {horizontal:?}"
    );
    println!("[PASS] a 2-D radius finds both ends of a 100 m vertical pair");

    // And the vertical gap is measurable, which is the point of keeping z. The
    // ground reference is a literal at the same lon/lat and z = 0, so the whole of
    // the 100 m has to come from the STORED altitude.
    let ground = format!(
        "ST_GEOMFROMGEOJSON('{{\"type\":\"Point\",\"coordinates\":[{},{},0]}}')",
        CENTER_LON - 0.000_05,
        north(5.0)
    );
    let d3 = sql_scalar(
        base,
        token,
        &format!(
            "SELECT ST_3DDISTANCE(CAST(properties->>'location' AS GEOMETRY), {ground}) AS d \
             FROM '{WORKSPACE}' WHERE path = '/tower-top'"
        ),
        "d",
    )
    .await;
    let d3 = d3
        .as_f64()
        .unwrap_or_else(|| panic!("ST_3DDISTANCE returned {d3}"));
    assert!(
        (d3 - 100.0).abs() < 0.5,
        "the stored point is 100 m above a co-located ground reference, so the 3-D \
         distance is 100 m; got {d3}"
    );
    println!("[PASS] ST_3DDISTANCE against stored altitude = {d3:.3} m");
}

// --- fixtures ------------------------------------------------------------------

/// The health check the harness waits on is satisfied by any listener on the port,
/// so a port collision looks like success. If our own child has already exited, we
/// are talking to somebody else's database and every assertion below is meaningless.
fn assert_server_is_ours(server: &mut ServerHandle) {
    match server.process.try_wait() {
        Ok(Some(status)) => panic!(
            "the server process exited with {status} while /health answered — port \
             {PORT} is already in use by another test's server, so this test would \
             have run against a foreign database"
        ),
        Ok(None) => {}
        Err(e) => panic!("could not poll the server process: {e}"),
    }
}

async fn bootstrap_admin(base_url: &str) -> String {
    let token = authenticate(base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("authenticate");

    let client = Client::new();
    let profile = client
        .get(format!("{base_url}/api/raisindb/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap().to_string();

    client
        .put(format!(
            "{base_url}/api/raisindb/sys/default/users/{user_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await
        .unwrap();

    authenticate(base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("re-authenticate")
}

async fn provision(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "Spatial index lifecycle test repo",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WORKSPACE}"),
        token,
        json!({
            "name": WORKSPACE,
            "description": "Places with indoor-scale geometry",
            // `raisin:Folder` must be allowed: creating a workspace materialises a
            // root folder node, so a workspace permitting only its own type is
            // rejected at creation time.
            "allowed_node_types": ["geo:Place", "raisin:Folder"],
            "allowed_root_node_types": ["geo:Place", "raisin:Folder"],
            "depends_on": [],
            "config": {
                "default_branch": BRANCH,
                "node_type_pins": {},
                // Replicated intent: the sibling property whose value discriminates
                // candidates inside a cell. Purely a selectivity device — the
                // predicate it comes from always stays in the residual filter.
                "spatial": { "default": { "bucket_property": "floor" }, "properties": {} }
            }
        }),
    )
    .await
    .expect("create workspace");

    http_post(
        base_url,
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        token,
        json!({
            "node_type": {
                "name": "geo:Place",
                "description": "A place with an indoor-scale location",
                "properties": [
                    { "name": "title", "type": "String", "required": true },
                    { "name": "floor", "type": "String" },
                    { "name": "location", "type": "Geometry" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create geo:Place NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}
