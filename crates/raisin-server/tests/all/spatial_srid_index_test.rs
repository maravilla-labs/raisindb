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

//! Write-time SRID normalisation of the SPATIAL INDEX, end to end.
//!
//! `spatial_crs_test` proves that a geometry keeps its SRID through storage and
//! that `ST_TRANSFORM` is arithmetically right. This file proves the thing that
//! sits *between* those two and used to be missing entirely: that a geometry in
//! a projected CRS actually lands in the geohash index, at cells a WGS84 query
//! can find.
//!
//! # The bug
//!
//! Geohash cells are lon/lat degrees. Zurich in EPSG:3857 is
//! `(950_668, 6_002_678)`, which fails the `-180..=180 / -90..=90` domain check,
//! so the encoder produced **no cell at all**. The row was stored, reported
//! healthy, and was invisible to every `ST_DWITHIN` forever, with nothing logged.
//! `raisin_proj::normalize_for_index` existed and was unit-tested but had zero
//! callers. Phase 2 below is the assertion that would have caught it: an index
//! census of six nodes when only three are in WGS84.
//!
//! # Why unit tests were not enough
//!
//! The normaliser is called from `cells_for_geometry`, several layers under an
//! HTTP request. Proving it in isolation says nothing about whether the SQL DML
//! path, the REST node path and the index writer all reach it — which is exactly
//! the class of gap this subsystem keeps having. Everything here goes over the
//! wire to a real server, and writes over BOTH SQL and REST.
//!
//! Run with:
//! `cargo test -p raisin-server --test all spatial_srid_index_test -- --ignored --nocapture`

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const TENANT: &str = "default";
const REPO: &str = "srid_index_test";
const BRANCH: &str = "main";
const WORKSPACE: &str = "sites";
const PORT: u16 = 8121;

/// The default precision set, `INDEX_PRECISIONS_DEFAULT` — eight entries per
/// geometry per revision.
const PRECISIONS: usize = 8;

/// Zurich, and the same point in EPSG:3857 and EPSG:32632 (WGS84 UTM 32N).
/// Externally derived; see `spatial_crs_test`'s header for the derivation.
const ZURICH: (f64, f64) = (8.54, 47.37);
const ZURICH_3857: (f64, f64) = (950_668.451_374_556_3, 6_002_677.997_532_715);
const ZURICH_UTM32N: (f64, f64) = (465_270.423_099_666_7, 5_246_384.775_981_838);

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

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

async fn http_put(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let response = Client::new()
        .put(format!("{base_url}{path}"))
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
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn sql(base_url: &str, token: &str, statement: &str) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{REPO}"),
        token,
        json!({ "sql": statement, "params": [] }),
    )
    .await
}

fn rows(response: &Value) -> Vec<Value> {
    response["rows"].as_array().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

async fn bootstrap_admin(base_url: &str) -> String {
    let token = authenticate(base_url, TENANT, "admin", "Admin12345!@#")
        .await
        .expect("authenticate");
    let client = Client::new();
    let profile: Value = client
        .get(format!("{base_url}/api/raisindb/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap().to_string();
    let _ = client
        .put(format!(
            "{base_url}/api/raisindb/sys/{TENANT}/users/{user_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await;
    authenticate(base_url, TENANT, "admin", "Admin12345!@#")
        .await
        .expect("re-authenticate")
}

async fn provision(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({ "repo_id": REPO, "description": "SRID index normalisation", "default_branch": BRANCH }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WORKSPACE}"),
        token,
        json!({
            "name": WORKSPACE,
            "description": "sites in assorted CRS",
            // `raisin:Folder` must be allowed as a root type: workspace creation
            // materialises a root folder node and is rejected without it.
            "allowed_node_types": ["crs:Site", "raisin:Folder"],
            "allowed_root_node_types": ["crs:Site", "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
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
                "name": "crs:Site",
                "description": "A site with a geometry in some CRS",
                "properties": [
                    { "name": "title", "type": "String", "required": true },
                    { "name": "geom", "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "create crs:Site", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}

/// A second workspace holding EPSG:3857 sites only.
///
/// Needed because the row-level `ST_DWITHIN` refuses to compare two different
/// explicit SRIDs (`resolve_pair_srid`, deliberately an error rather than an
/// implicit transform), so a projected query centre cannot be asked against the
/// mixed-CRS workspace at all.
const MERCATOR_WS: &str = "mercator_sites";

async fn provision_mercator_workspace(base_url: &str, token: &str) {
    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{MERCATOR_WS}"),
        token,
        json!({
            "name": MERCATOR_WS,
            "description": "sites stored exclusively in EPSG:3857",
            "allowed_node_types": ["crs:Site", "raisin:Folder"],
            "allowed_root_node_types": ["crs:Site", "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create mercator workspace");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}

/// A constant point in a projected CRS, spelled as a GeoJSON literal.
///
/// `ST_POINT(950668, 6002678)` is NOT usable here: the constructor validates its
/// arguments as lon/lat and rejects an easting, whatever `ST_SETSRID` is wrapped
/// around it afterwards. A labelled GeoJSON literal is the supported spelling for
/// a projected constant.
fn projected_point(x: f64, y: f64, srid: u32) -> String {
    format!(
        "ST_GEOMFROMGEOJSON('{{\"type\":\"Point\",\"coordinates\":[{x},{y}],\"srid\":{srid}}}')"
    )
}

/// A GeoJSON Point literal with an explicit SRID, as a JSON string for `::jsonb`.
fn site_properties(title: &str, x: f64, y: f64, srid: Option<u32>) -> String {
    let geom = match srid {
        Some(s) => json!({"type": "Point", "coordinates": [x, y], "srid": s}),
        None => json!({"type": "Point", "coordinates": [x, y]}),
    };
    json!({ "title": title, "geom": geom })
        .to_string()
        .replace('\'', "''")
}

/// Insert over **SQL DML**, the path that previously did not even produce a
/// `PropertyValue::Geometry`.
async fn insert_via_sql(
    base_url: &str,
    token: &str,
    id: &str,
    x: f64,
    y: f64,
    srid: Option<u32>,
) -> Result<Value, String> {
    insert_via_sql_into(base_url, token, WORKSPACE, id, x, y, srid).await
}

/// As [`insert_via_sql`], into an explicit workspace.
async fn insert_via_sql_into(
    base_url: &str,
    token: &str,
    workspace: &str,
    id: &str,
    x: f64,
    y: f64,
    srid: Option<u32>,
) -> Result<Value, String> {
    let props = site_properties(id, x, y, srid);
    sql(
        base_url,
        token,
        &format!(
            "INSERT INTO '{workspace}' (id, name, node_type, path, properties) VALUES \
             ('{id}', '{id}', 'crs:Site', '/{id}', '{props}'::jsonb)"
        ),
    )
    .await
}

/// Insert over the **REST node API**, the other write path.
async fn insert_via_rest(
    base_url: &str,
    token: &str,
    id: &str,
    x: f64,
    y: f64,
    srid: Option<u32>,
) -> Result<Value, String> {
    let geom = match srid {
        Some(s) => json!({"type": "Point", "coordinates": [x, y], "srid": s}),
        None => json!({"type": "Point", "coordinates": [x, y]}),
    };
    http_post(
        base_url,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/"),
        token,
        json!({
            "node": {
                "id": id,
                "name": id,
                "node_type": "crs:Site",
                "properties": { "title": id, "geom": geom }
            }
        }),
    )
    .await
}

/// The single spatial HEALTH row for `sites`.`geom`.
async fn health(base_url: &str, token: &str) -> Value {
    let r = sql(
        base_url,
        token,
        &format!("SHOW SPATIAL INDEX HEALTH FOR '{WORKSPACE}' PROPERTY 'geom'"),
    )
    .await
    .expect("SHOW SPATIAL INDEX HEALTH");
    rows(&r).into_iter().next().unwrap_or(Value::Null)
}

/// Poll until the census reports `nodes` distinct indexed nodes.
///
/// Polled rather than slept: the reconcile job may be rewriting entries
/// concurrently with the inserts, so a fixed sleep made this flaky.
async fn await_indexed(base_url: &str, token: &str, nodes: usize, what: &str) -> Value {
    let want_live = (nodes * PRECISIONS) as i64;
    for attempt in 0..60 {
        let row = health(base_url, token).await;
        if row["distinct_nodes"].as_i64() == Some(nodes as i64)
            && row["live_entries"].as_i64() == Some(want_live)
        {
            println!(
                "[OK] {what} after {} poll(s): distinct_nodes={} live_entries={}",
                attempt + 1,
                row["distinct_nodes"],
                row["live_entries"]
            );
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!(
        "{what}: timed out waiting for {nodes} indexed nodes ({want_live} live entries); \
         last health = {}",
        health(base_url, token).await
    );
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_srid_index_test -- --ignored --nocapture
async fn test_projected_geometries_are_indexed_in_wgs84() {
    println!("\n=== write-time SRID normalisation of the spatial index ===\n");

    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let token = bootstrap_admin(&server.base_url).await;
    let base = server.base_url.clone();
    provision(&base, &token).await;
    println!("[OK] server up, repo/workspace/nodetype provisioned");

    // -------------------------------------------------- 1. write, both paths

    println!("\n--- 1. six sites: three CRS x two write paths ---");
    insert_via_sql(&base, &token, "sql-wgs84", ZURICH.0, ZURICH.1, None)
        .await
        .expect("SQL insert, unlabelled 4326");
    insert_via_sql(
        &base,
        &token,
        "sql-mercator",
        ZURICH_3857.0,
        ZURICH_3857.1,
        Some(3857),
    )
    .await
    .expect("SQL insert, EPSG:3857");
    insert_via_sql(
        &base,
        &token,
        "sql-utm",
        ZURICH_UTM32N.0,
        ZURICH_UTM32N.1,
        Some(32632),
    )
    .await
    .expect("SQL insert, EPSG:32632");

    insert_via_rest(&base, &token, "rest-wgs84", ZURICH.0, ZURICH.1, Some(4326))
        .await
        .expect("REST insert, explicit 4326");
    insert_via_rest(
        &base,
        &token,
        "rest-mercator",
        ZURICH_3857.0,
        ZURICH_3857.1,
        Some(3857),
    )
    .await
    .expect("REST insert, EPSG:3857");
    insert_via_rest(
        &base,
        &token,
        "rest-utm",
        ZURICH_UTM32N.0,
        ZURICH_UTM32N.1,
        Some(32632),
    )
    .await
    .expect("REST insert, EPSG:32632");
    println!("[OK] 6 sites written (3 over SQL DML, 3 over the REST node API)");

    // ------------------------------------------ 2. THE regression assertion

    println!("\n--- 2. every site is in the index, projected or not ---");
    let row = await_indexed(&base, &token, 6, "all six sites indexed").await;
    assert_eq!(
        row["distinct_nodes"].as_i64(),
        Some(6),
        "projected geometries must be indexed too; before write-time normalisation \
         only the WGS84 pair produced cells and this reported 2"
    );
    println!("[PASS] index census: 6 nodes x {PRECISIONS} precisions");

    // ------------------------------------ 3. the cells are in the WGS84 frame

    // Being *in* the index is not enough — the cells must be where a WGS84 query
    // looks. All six sites are the same physical place, so a tight radius around
    // Zurich must return all six. A geometry geohashed from raw eastings would be
    // in the index and still never match here.
    println!("\n--- 3. a WGS84 radius query finds all six ---");
    let found = sql(
        &base,
        &token,
        &format!(
            "SELECT name FROM {WORKSPACE} \
             WHERE ST_DWITHIN(properties->>'geom', ST_POINT({}, {}), 50) \
             ORDER BY name",
            ZURICH.0, ZURICH.1
        ),
    )
    .await
    .expect("ST_DWITHIN query");
    let names: Vec<String> = rows(&found)
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_string))
        .collect();
    println!("  matched: {names:?}");
    assert_eq!(
        names.len(),
        6,
        "all six sites are the same place, so a 50 m radius must return all of \
         them; got {names:?}"
    );

    // ---------------------------------------- 4. a projected centre still works

    // The query planner declines the index for a projected centre — its radius is
    // in projected metres, which are not ground metres, so an index scan planned
    // from it could be a SUBSET rather than a superset. The predicate stays in the
    // residual filter and the SRID-aware row evaluator answers it. That must
    // ANSWER, not error: before the SRID gate the centre's label was dropped
    // during constant folding, 950668 was planned as a longitude, and the cell
    // planner failed the whole query with a "cannot cover" message.
    //
    // Asked in a workspace whose sites are ALL EPSG:3857, because the row
    // evaluator (correctly) refuses to compare two different explicit SRIDs — the
    // mixed workspace above would raise `SRID mismatch (32632 vs 3857)`.
    println!("\n--- 4. a projected query centre falls back instead of failing ---");
    provision_mercator_workspace(&base, &token).await;
    insert_via_sql_into(
        &base,
        &token,
        MERCATOR_WS,
        "m1",
        ZURICH_3857.0,
        ZURICH_3857.1,
        Some(3857),
    )
    .await
    .expect("SQL insert into the mercator-only workspace");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let projected_query = sql(
        &base,
        &token,
        &format!(
            "SELECT name FROM {MERCATOR_WS} \
             WHERE ST_DWITHIN(properties->>'geom', {}, 100) \
             ORDER BY name",
            projected_point(ZURICH_3857.0, ZURICH_3857.1, 3857)
        ),
    )
    .await
    .expect("a projected query centre must not fail the query");
    let projected_names: Vec<String> = rows(&projected_query)
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_string))
        .collect();
    println!("  matched: {projected_names:?}");
    assert_eq!(
        projected_names,
        vec!["m1".to_string()],
        "an EPSG:3857 centre must match the EPSG:3857 site over the same place"
    );

    // ------------------------------- 5. an unindexable SRID fails LOUDLY

    // EPSG:2056 (Swiss LV95) is a real CRS that `proj4rs-backend` and
    // `proj-backend` both know — and the index normaliser refuses on EVERY build,
    // because widening it by Cargo feature would let two nodes of one cluster hold
    // different index contents for the same replicated record. The write must
    // therefore FAIL rather than store a row no spatial query can ever find.
    println!("\n--- 5. an unindexable SRID is rejected, not silently unindexed ---");
    let via_sql = insert_via_sql(
        &base,
        &token,
        "sql-lv95",
        2_683_000.0,
        1_247_000.0,
        Some(2056),
    )
    .await
    .err()
    .expect("EPSG:2056 must be REJECTED at write time, not silently unindexed");
    println!("  SQL:  {via_sql}");
    assert!(
        via_sql.contains("2056"),
        "the error must name the offending SRID: {via_sql}"
    );

    let via_rest = insert_via_rest(
        &base,
        &token,
        "rest-lv95",
        2_683_000.0,
        1_247_000.0,
        Some(2056),
    )
    .await
    .err()
    .expect("EPSG:2056 must be rejected on the REST path too");
    println!("  REST: {via_rest}");
    assert!(
        via_rest.contains("2056"),
        "the error must name the offending SRID: {via_rest}"
    );

    // ...and nothing was stored, so the index census is unchanged.
    let after = health(&base, &token).await;
    assert_eq!(
        after["distinct_nodes"].as_i64(),
        Some(6),
        "a rejected write must leave no trace: {after}"
    );
    println!("[PASS] EPSG:2056 rejected on both write paths, index untouched");

    println!("\n=== write-time SRID normalisation: PASS ===\n");
}
