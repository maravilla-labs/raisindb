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

//! **SQL is the write interface.** This module proves, against a real server,
//! that geometry written with SQL `INSERT` / `UPDATE` / `DELETE` — including the
//! bulk/compound forms — populates and *maintains* the spatial index, so an
//! indexed `ST_DWITHIN` returns the truth afterwards.
//!
//! # Why the bulk cases are asserted separately rather than assumed
//!
//! An `UPDATE` or `DELETE` whose `WHERE` clause is not a bare `id =` / `path =`
//! equality is classified `FilterComplexity::Complex`
//! (`dml_executor/node_helpers.rs`) and, over HTTP, handed to the async `BulkSql`
//! job instead of executing inline (`engine/batch.rs`, `handlers/sql/engine.rs`).
//! That executor was once a debug stub that silently did nothing, so node-event
//! triggers never fired on compound SQL writes. Treating the bulk path as covered
//! by the single-row path would repeat exactly that mistake, so every bulk case
//! here is written and read back on its own.
//!
//! # What "the index is actually used" means here
//!
//! Correct rows alone do not prove the index ran: a spatial predicate that falls
//! back to a row-level filter also returns correct rows, just slowly — that is the
//! whole point of the fail-closed planner. So the row assertions are paired with an
//! `EXPLAIN` assertion that the plan is a `SpatialDistanceScan`. If the plan is a
//! fallback the test says so loudly rather than passing quietly.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{bootstrap_admin, provision, sql_http};
use serde_json::Value;
use std::time::Duration;

const REPO: &str = "sqlgeo";
const BRANCH: &str = "main";
const WS: &str = "shops";
const NODE_TYPE: &str = "geo:Shop";
const PORT: u16 = 8106;

/// Zurich Hauptbahnhof. Any lon/lat would do; a real place makes a failure
/// legible when it prints.
const CENTER: (f64, f64) = (8.5402, 47.3779);
/// ~0.00030 degrees of latitude is ~33 m — inside a 100 m radius, outside a 10 m one.
const NEAR_OFFSET: f64 = 0.000_30;
/// ~5 km north, far enough that no radius under test spans both.
const FAR_OFFSET: f64 = 0.045;

// ------------------------------------------------------------------ utilities

fn point_json(lon: f64, lat: f64) -> String {
    format!("{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}")
}

/// A `properties` JSONB literal for a shop.
fn props(title: &str, floor: &str, lon: f64, lat: f64) -> String {
    format!(
        "'{{\"title\":\"{title}\",\"floor\":\"{floor}\",\"location\":{}}}'::JSONB",
        point_json(lon, lat)
    )
}

fn dwithin_sql(lon: f64, lat: f64, radius: f64) -> String {
    format!(
        "SELECT name FROM '{WS}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({lon}, {lat}), {radius}) \
         ORDER BY name"
    )
}

fn names_of(result: &Value) -> Vec<String> {
    let mut out: Vec<String> = result["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

// --------------------------------------------------------------- the test body

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_sql_write_test -- --ignored --nocapture
async fn test_sql_dml_maintains_the_spatial_index() {
    println!("\n=== SQL DML -> spatial index, end to end ===\n");

    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    let base = server.base_url.clone();
    provision(&base, &token, REPO, BRANCH, WS, NODE_TYPE).await;
    println!("[OK] server up, repo/workspace/nodetype provisioned");

    let ctx = Ctx { base, token };

    single_row_insert(&ctx).await;
    index_is_actually_used(&ctx).await;
    single_row_update_moves_the_entry(&ctx).await;
    multi_row_insert(&ctx).await;
    bulk_update(&ctx).await;
    single_row_delete(&ctx).await;
    bulk_delete(&ctx).await;

    println!("\n=== SQL DML -> spatial index: PASS ===\n");
}

struct Ctx {
    base: String,
    token: String,
}

impl Ctx {
    async fn run(&self, sql: &str) -> Value {
        sql_http(&self.base, &self.token, REPO, sql)
            .await
            .unwrap_or_else(|e| panic!("SQL failed\n  {sql}\n  {e}"))
    }

    async fn try_run(&self, sql: &str) -> Result<Value, String> {
        sql_http(&self.base, &self.token, REPO, sql).await
    }

    async fn names(&self, sql: &str) -> Vec<String> {
        names_of(&self.run(sql).await)
    }

    /// Assert a spatial query's row set, retrying until it settles.
    ///
    /// Retries exist for the bulk paths only: over HTTP a complex `UPDATE` /
    /// `DELETE` is accepted as a job and applied a moment later, so a single
    /// immediate read would be racing the queue rather than testing the index.
    async fn expect_names(&self, label: &str, sql: &str, want: &[&str]) {
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        let deadline = std::time::Instant::now() + Duration::from_secs(45);
        let mut got;
        loop {
            got = self.names(sql).await;
            if got == want || std::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        assert_eq!(got, want, "{label}\n  query: {sql}");
        println!("[PASS] {label} -> {got:?}");
    }
}

// ------------------------------------------------------------------- 1. INSERT

async fn single_row_insert(ctx: &Ctx) {
    println!("\n--- 1. single-row SQL INSERT ---");
    let (lon, lat) = (CENTER.0, CENTER.1 + NEAR_OFFSET);
    ctx.run(&format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) \
         VALUES ('s1', '/s1', '{NODE_TYPE}', {})",
        props("Shop One", "L1", lon, lat)
    ))
    .await;

    // The row must be reachable *through the spatial predicate*, not merely stored.
    ctx.expect_names(
        "INSERT then ST_DWITHIN(100m) finds the row",
        &dwithin_sql(CENTER.0, CENTER.1, 100.0),
        &["s1"],
    )
    .await;

    // And the radius must actually discriminate — a query that returns everything
    // proves nothing about the index.
    ctx.expect_names(
        "a 10m radius excludes a shop 33m away",
        &dwithin_sql(CENTER.0, CENTER.1, 10.0),
        &[],
    )
    .await;
}

// ---------------------------------------------------- 2. the index really runs

async fn index_is_actually_used(ctx: &Ctx) {
    println!("\n--- 2. EXPLAIN: the predicate is served by the spatial index ---");
    let explain = ctx
        .run(&format!(
            "EXPLAIN {}",
            dwithin_sql(CENTER.0, CENTER.1, 100.0)
        ))
        .await;
    let plan = explain["explain_plan"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            explain["rows"][0]["QUERY PLAN"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{explain}"));

    println!("{plan}");
    assert!(
        plan.contains("SpatialDistanceScan"),
        "the spatial predicate must reach the index after a SQL INSERT. A plan \
         without SpatialDistanceScan means the SQL write path did not populate \
         the index (or the state record says NOT BUILT), and every row assertion \
         in this module is then only proving the row-level fallback:\n{plan}"
    );
    println!("[PASS] plan is a SpatialDistanceScan — the SQL write populated the index");
}

// ------------------------------------------------------------------- 3. UPDATE

async fn single_row_update_moves_the_entry(ctx: &Ctx) {
    println!("\n--- 3. single-row SQL UPDATE moves the index entry ---");
    let (new_lon, new_lat) = (CENTER.0, CENTER.1 + FAR_OFFSET);
    ctx.run(&format!(
        "UPDATE '{WS}' SET properties = {} WHERE id = 's1'",
        props("Shop One", "L1", new_lon, new_lat)
    ))
    .await;

    // The old cell must stop matching. This is the half that a stale-entry bug
    // gets wrong: a moved node matching at BOTH locations.
    ctx.expect_names(
        "after UPDATE the OLD location no longer matches",
        &dwithin_sql(CENTER.0, CENTER.1, 200.0),
        &[],
    )
    .await;
    ctx.expect_names(
        "after UPDATE the NEW location matches",
        &dwithin_sql(new_lon, new_lat, 200.0),
        &["s1"],
    )
    .await;
}

// ------------------------------------------------------- 4. bulk INSERT (one stmt)

/// Four shops written by a SINGLE multi-row `INSERT ... VALUES (..),(..),(..),(..)`.
/// Offsets put them at roughly 55 m, 110 m, 550 m and 5.5 km from `CENTER`.
const BULK: [(&str, f64); 4] = [
    ("m1", 0.000_5),
    ("m2", 0.001_0),
    ("m3", 0.005_0),
    ("m4", 0.050_0),
];

async fn multi_row_insert(ctx: &Ctx) {
    println!("\n--- 4. compound SQL INSERT: four rows, one statement ---");
    let values = BULK
        .iter()
        .map(|(id, off)| {
            format!(
                "('{id}', '/{id}', '{NODE_TYPE}', {})",
                props(id, "L2", CENTER.0, CENTER.1 + off)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    ctx.run(&format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES {values}"
    ))
    .await;

    // Each row must be findable at its own scale, which a single wide radius
    // would not distinguish from "the index returned everything".
    ctx.expect_names(
        "compound INSERT: 100m finds only m1",
        &dwithin_sql(CENTER.0, CENTER.1, 100.0),
        &["m1"],
    )
    .await;
    ctx.expect_names(
        "compound INSERT: 200m finds m1+m2",
        &dwithin_sql(CENTER.0, CENTER.1, 200.0),
        &["m1", "m2"],
    )
    .await;
    ctx.expect_names(
        "compound INSERT: 1km finds m1+m2+m3",
        &dwithin_sql(CENTER.0, CENTER.1, 1000.0),
        &["m1", "m2", "m3"],
    )
    .await;
    // s1 sits ~5.0 km north, m4 ~5.55 km north: a 10 km radius must find all five.
    ctx.expect_names(
        "compound INSERT: 10km finds every shop",
        &dwithin_sql(CENTER.0, CENTER.1, 10_000.0),
        &["m1", "m2", "m3", "m4", "s1"],
    )
    .await;
}

// ---------------------------------------------------------------- 5. bulk UPDATE

/// A place far from every other fixture, so "did the bulk update move the index
/// entries" cannot be confused with "the old entries are still there".
const BULK_TARGET: (f64, f64) = (8.6, 47.6);

async fn bulk_update(ctx: &Ctx) {
    println!("\n--- 5. bulk SQL UPDATE (complex WHERE -> BulkSql job) ---");
    let sql = format!(
        "UPDATE '{WS}' SET properties = {} WHERE node_type = '{NODE_TYPE}'",
        props("relocated", "L9", BULK_TARGET.0, BULK_TARGET.1)
    );
    let response = ctx.run(&sql).await;
    if let Some(job_id) = response["rows"][0]["job_id"].as_str() {
        println!("  routed to the async BulkSql job {job_id} (complex WHERE)");
    } else {
        println!("  executed inline: {}", response["rows"]);
    }

    // Every shop must now be at the new place...
    ctx.expect_names(
        "bulk UPDATE moved every row to the new location",
        &dwithin_sql(BULK_TARGET.0, BULK_TARGET.1, 200.0),
        &["m1", "m2", "m3", "m4", "s1"],
    )
    .await;
    // ...and nowhere near the old ones. This is the assertion the silent-no-op
    // BulkSql stub would have failed.
    ctx.expect_names(
        "bulk UPDATE left no stale entries at the old locations",
        &dwithin_sql(CENTER.0, CENTER.1, 10_000.0),
        &[],
    )
    .await;
}

// ---------------------------------------------------------------- 6/7. DELETE

async fn single_row_delete(ctx: &Ctx) {
    println!("\n--- 6. single-row SQL DELETE ---");
    ctx.run(&format!("DELETE FROM '{WS}' WHERE id = 's1'"))
        .await;
    ctx.expect_names(
        "DELETE by id removes the row from spatial results",
        &dwithin_sql(BULK_TARGET.0, BULK_TARGET.1, 200.0),
        &["m1", "m2", "m3", "m4"],
    )
    .await;
}

async fn bulk_delete(ctx: &Ctx) {
    println!("\n--- 7. bulk SQL DELETE (complex WHERE) ---");
    let sql = format!("DELETE FROM '{WS}' WHERE node_type = '{NODE_TYPE}'");
    match ctx.try_run(&sql).await {
        Ok(response) => {
            if let Some(job_id) = response["rows"][0]["job_id"].as_str() {
                println!("  routed to the async BulkSql job {job_id}");
            }
            ctx.expect_names(
                "bulk DELETE removes every row from spatial results",
                &dwithin_sql(BULK_TARGET.0, BULK_TARGET.1, 200.0),
                &[],
            )
            .await;
        }
        Err(e) => {
            // Reported, never swallowed: a bulk DELETE that cannot even be
            // submitted is a finding about the DML surface, not about spatial.
            panic!(
                "bulk DELETE was rejected outright — this is a real gap, not a test artefact: {e}"
            );
        }
    }
}
