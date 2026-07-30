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

//! Sections 3–5 — row semantics, distance ordering, and the unindexed path.
//!
//! # "If there are n geospatial in one node would I get all?"
//!
//! An **index-backed** query names exactly ONE path, so it searches one field's
//! entries and a node yields at most one row. The only shape in which one node
//! can match through several of its geometries at once is the explicit `[]`
//! wildcard, and the rule there is: still ONE row, and `ST_DISTANCE` is the
//! **minimum** over the matched geometries.
//!
//! The minimum is not a taste call. It is what makes
//! `ORDER BY ST_DISTANCE(...) LIMIT k` mean "the k nearest nodes", and
//! one-row-per-node is what stops a keyset cursor on distance from straddling two
//! rows of the same node and both duplicating and skipping at page boundaries.

use super::fixture::Ctx;
use super::{dwithin, LAT, LON_C0_PIN, LON_C0_STAGE, LON_C1_PIN, LON_HERO_STAGE, WS};

/// Midway between `content.0.pin` (8.80) and `content.1.pin` (9.00).
const BETWEEN_PINS: f64 = 8.85;

/// `ST_DISTANCE` on a property path, as a single scalar column `d`.
fn distance_to(path: &str, lon: f64, lat: f64, id: &str) -> String {
    format!(
        "SELECT ST_DISTANCE(CAST(properties->>'{path}' AS GEOMETRY), \
                ST_POINT({lon}, {lat})) AS d \
         FROM '{WS}' WHERE id = '{id}'"
    )
}

/// A node whose radius covers TWO of its own geometries yields ONE row, and the
/// distance reported is the nearer of the two.
pub async fn one_row_per_node_and_minimum_distance(ctx: &Ctx) {
    println!("\n--- 3. one row per node; the wildcard distance is the minimum ---");

    // A 20 km radius from midway between v1's two content pins covers BOTH
    // (3.8 km and 11.3 km away) and reaches neither of v2's, 55 km north.
    let query = dwithin("content[].pin", BETWEEN_PINS, LAT, 20_000.0);
    let rows = ctx.names(&query).await;
    assert_eq!(
        rows,
        vec!["v1".to_string()],
        "a radius covering TWO of one node's geometries must yield exactly ONE row \
         — one row per geometry would straddle keyset page boundaries and both \
         duplicate and skip rows. Got {rows:?}\n  query: {query}"
    );
    println!("[PASS] two matching geometries on one node -> exactly one row");

    // Which of the two distances is reported: the MINIMUM. First-found or maximum
    // would give ~11.3 km.
    let d = ctx
        .scalar(&distance_to("content[].pin", BETWEEN_PINS, LAT, "v1"), "d")
        .await
        .expect("wildcard distance");
    assert!(
        (3_000.0..5_000.0).contains(&d),
        "the wildcard distance must be the MINIMUM over the node's matching \
         geometries (~3.8 km to content.0.pin), got {d:.0} m — ~11.3 km would mean \
         maximum or first-found, and either breaks ORDER BY … LIMIT k"
    );
    println!("[PASS] wildcard distance is the minimum: {d:.0} m");

    // …and it really is a minimum over the SET, not a fixed pick: asked from the
    // far side, the OTHER element becomes the nearest.
    let d_far = ctx
        .scalar(&distance_to("content[].pin", LON_C1_PIN, LAT, "v1"), "d")
        .await
        .expect("wildcard distance from the far pin");
    assert!(
        d_far < 500.0,
        "asked at content.1.pin's own position the minimum must be ~0 m, got \
         {d_far:.0} m — a fixed pick of the first element would report ~15 km"
    );
    println!("[PASS] the minimum tracks the query point: {d_far:.0} m at the second pin");

    // A named concrete path is unambiguous by construction: it reports THAT
    // element's distance, not the node's nearest geometry.
    let d0 = ctx
        .scalar(&distance_to("content.0.pin", BETWEEN_PINS, LAT, "v1"), "d")
        .await
        .expect("concrete distance");
    let d1 = ctx
        .scalar(&distance_to("content.1.pin", BETWEEN_PINS, LAT, "v1"), "d")
        .await
        .expect("concrete distance");
    assert!(
        d0 < d1 && (d1 - d0) > 5_000.0,
        "naming a concrete array element must report THAT element's distance: \
         content.0.pin={d0:.0} m, content.1.pin={d1:.0} m"
    );
    println!("[PASS] concrete element paths report their own distance: {d0:.0} m vs {d1:.0} m");
}

/// `ORDER BY ST_DISTANCE(...) LIMIT k` stays correct when a node matches through
/// several geometries.
pub async fn distance_ordering_with_several_matching_geometries(ctx: &Ctx) {
    println!("\n--- 4. ORDER BY ST_DISTANCE … LIMIT with several matching geometries ---");

    let knn = |path: &str, lon: f64, lat: f64, k: usize| {
        format!(
            "SELECT name FROM '{WS}' \
             ORDER BY ST_DISTANCE(CAST(properties->>'{path}' AS GEOMETRY), \
                                  ST_POINT({lon}, {lat})) LIMIT {k}"
        )
    };

    // v1 matches through two content pins (3.8 km and 11.3 km); v2 through two of
    // its own, both ~56 km away. Nearest-first must therefore be v1 then v2, and
    // the per-node minimum is what makes that a total order over NODES.
    let one = ctx.names(&knn("content[].pin", BETWEEN_PINS, LAT, 1)).await;
    assert_eq!(
        one,
        vec!["v1".to_string()],
        "LIMIT 1 must return the nearest NODE — with a per-geometry row model the \
         same node would occupy both slots. Got {one:?}"
    );
    let two = ctx.names(&knn("content[].pin", BETWEEN_PINS, LAT, 2)).await;
    assert_eq!(
        two,
        vec!["v1".to_string(), "v2".to_string()],
        "LIMIT k is k NODES, nearest-first, not k geometries. Got {two:?}"
    );
    println!("[PASS] wildcard k-NN is per node and nearest-first: {two:?}");

    // The wildcard must NOT claim the scan's own ordering: the minimum over
    // several geometries is not the order any single cell-ring scan produces, so
    // an explicit Sort has to survive. Getting this wrong looks fine in small
    // tests and drops/duplicates rows under keyset pagination.
    let wildcard_plan = ctx
        .explain(&knn("content[].pin", BETWEEN_PINS, LAT, 2))
        .await;
    assert!(
        !wildcard_plan.contains("SpatialKnnScan"),
        "a wildcard distance order must not be served by a k-NN cell scan — each \
         array element is indexed under its own concrete path, so the wildcard \
         prefix holds nothing:\n{wildcard_plan}"
    );
    println!("[PASS] the wildcard k-NN keeps an explicit sort (no SpatialKnnScan)");

    // A concrete nested path, by contrast, IS index-ordered.
    let concrete = knn("hero.stage.geo", LON_HERO_STAGE, LAT, 2);
    let plan = ctx.explain(&concrete).await;
    assert!(
        plan.contains("SpatialKnnScan"),
        "a concrete 3-level path must be served by the k-NN scan; without it the \
         nested path never reached the index:\n{plan}"
    );
    assert_eq!(
        ctx.names(&concrete).await,
        vec!["v1".to_string(), "v2".to_string()],
        "nearest-first on a 3-level nested path"
    );
    println!("[PASS] a 3-level nested path plans as a SpatialKnnScan and orders correctly");
}

/// A path with no index must be answered by a row scan that is **correct**.
///
/// This is the highest-risk item of the whole area. Before the row-level dotted
/// path resolver existed, `properties->>'hero.stage.geo'` was an ordinary JSON key
/// lookup, found no such key, evaluated to NULL, and the fallback — the thing
/// that is supposed to be "slow but correct" — was slow and returned NOTHING while
/// logging that it was fine. That is the window every nested query passes through
/// before a rebuild drains.
pub async fn an_unindexed_path_is_slow_but_never_empty(ctx: &Ctx) {
    println!("\n--- 5. an unindexed path: slow, correct, never silently empty ---");

    // A wildcard is never index-backed, by construction, on any machine in any
    // index state — so this forces the row-level evaluator deterministically.
    let query = dwithin("content[].stage.geo", LON_C0_STAGE, LAT, 1_000.0);
    let plan = ctx.explain(&query).await;
    assert!(
        !plan.contains("SpatialDistanceScan"),
        "a wildcard path must not be planned as an index scan:\n{plan}"
    );
    ctx.expect_names(
        "a 4-level wildcard path finds the node through the ROW-LEVEL filter",
        &query,
        &["v1"],
    )
    .await;

    // And it is a filter, not a pass-through: the same wildcard at a nearby but
    // wrong centre matches nothing.
    ctx.expect_names(
        "the row-level filter discriminates",
        &dwithin("content[].stage.geo", LON_C0_PIN, LAT, 1_000.0),
        &[],
    )
    .await;

    // A path no node carries is an empty result, not an error — an error here
    // would fail a user who queried a nested field before the rebuild drained.
    let unknown = ctx
        .try_run(&dwithin("hero.stage.nowhere", LON_HERO_STAGE, LAT, 1_000.0))
        .await;
    assert!(
        unknown.is_ok(),
        "an unknown nested path must return no rows, not an error: {unknown:?}"
    );
    println!("[PASS] an unknown nested path is an empty result, not an error");
}
