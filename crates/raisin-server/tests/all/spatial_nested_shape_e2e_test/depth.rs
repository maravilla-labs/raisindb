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

//! Section 1 — every depth is reachable, index-backed, and its own namespace.

use super::fixture::Ctx;
use super::{
    dwithin, dwithin_in, LAT, LON_HERO_PIN, LON_HERO_STAGE, LON_LOCATION, PATHS, V2_LAT,
    WS_NESTED_ONLY,
};

/// Each of the six paths finds its own node at its own position, is served by the
/// spatial index, and finds NOTHING at any sibling path's position.
///
/// The last part is what makes the rest non-vacuous. All six geometries live on
/// the same node, so a walker that collapsed nested paths onto their root — or a
/// query that ignored the path and searched every geometry — would return `v1`
/// for all thirty cross-checks.
pub async fn every_nested_path_is_index_backed_and_independent(ctx: &Ctx) {
    println!("\n--- 1. every nested depth: findable, index-backed, independent ---");

    for (path, lon) in PATHS {
        let query = dwithin(path, lon, LAT, 1_000.0);

        // Rows AND plan. Rows alone cannot distinguish an index hit from a full
        // scan that re-applied the predicate, and the whole point of this pass is
        // that nested geometry reaches the INDEX.
        ctx.expect_names(
            &format!("'{path}' finds v1 at its own position"),
            &query,
            &["v1"],
        )
        .await;
        ctx.expect_index_backed(&format!("'{path}'"), &query).await;

        // The same path on the northern node — proving the entries are per node,
        // not a workspace-wide smear.
        ctx.expect_names(
            &format!("'{path}' finds v2 at the northern position"),
            &dwithin(path, lon, V2_LAT, 1_000.0),
            &["v2"],
        )
        .await;
    }

    // The cross-check matrix: every path queried at every OTHER path's position
    // must be empty. 0.10° of longitude is ~7.5 km here, so a 1 km radius cannot
    // reach a neighbour by accident.
    let mut checked = 0;
    for (path, _) in PATHS {
        for (other, other_lon) in PATHS {
            if path == other {
                continue;
            }
            let got = ctx.names(&dwithin(path, other_lon, LAT, 1_000.0)).await;
            assert!(
                got.is_empty(),
                "'{path}' must not match at '{other}'s position ({other_lon}) — the \
                 two paths would have to share an index namespace for this to \
                 happen. Got {got:?}"
            );
            checked += 1;
        }
    }
    println!("[PASS] {checked} cross-path checks: naming one field searches only that field");
}

/// A node carrying BOTH a top-level and several nested geometries: each is
/// findable on its own, and neither shadows the other.
///
/// Before this pass the top-level one was indexed and the nested ones were not,
/// so a node like `v1` answered correctly for `location` and silently emptily for
/// everything else — which is precisely why "the top-level one still works" was
/// never evidence of anything.
pub async fn top_level_and_nested_on_one_node_are_independent(ctx: &Ctx) {
    println!("\n--- 2. one node, top-level AND nested geometry, independently findable ---");

    ctx.expect_names(
        "top-level 'location' finds v1",
        &dwithin("location", LON_LOCATION, LAT, 1_000.0),
        &["v1"],
    )
    .await;
    ctx.expect_names(
        "top-level 'location' does NOT answer for the 2-level path's position",
        &dwithin("location", LON_HERO_PIN, LAT, 1_000.0),
        &[],
    )
    .await;
    ctx.expect_names(
        "the 3-level 'hero.stage.geo' finds the SAME node at its own position",
        &dwithin("hero.stage.geo", LON_HERO_STAGE, LAT, 1_000.0),
        &["v1"],
    )
    .await;
    ctx.expect_names(
        "the 3-level path does NOT answer for the top-level position",
        &dwithin("hero.stage.geo", LON_LOCATION, LAT, 1_000.0),
        &[],
    )
    .await;

    // A radius wide enough to span the top-level geometry AND both hero
    // geometries still returns ONE row per query, because a query names ONE path.
    // (`hero.pin` is 7.5 km from `location` and 7.5 km from `hero.stage.geo`.)
    let wide = ctx
        .names(&dwithin("hero.pin", LON_HERO_PIN, LAT, 20_000.0))
        .await;
    assert_eq!(
        wide,
        vec!["v1".to_string()],
        "a 20 km radius on 'hero.pin' spans the node's other geometries too, but a \
         named path searches only its own entries and a node yields one row: {wide:?}"
    );
    println!("[PASS] a wide radius on one named path still yields exactly one row per node");
}

/// A node whose ONLY geometry is nested must be indexed like any other.
///
/// This is the other half of the flat-property-scan bug, and the more damaging
/// half. The write path decided *whether to index at all* by asking whether any
/// TOP-LEVEL property was a `Geometry`. A node modelled as "a page with a map
/// section" answers no, so it was skipped entirely — no entries, no state record,
/// nothing — and a whole workspace shaped that way was not partially indexed but
/// wholly unindexed, while `SHOW SPATIAL INDEX HEALTH` had nothing to report
/// because there was no record to report on.
///
/// `s1` lives in its own workspace so the assertion cannot be rescued by a
/// sibling node that happens to carry a top-level geometry.
pub async fn a_node_whose_only_geometry_is_nested_is_indexed(ctx: &Ctx) {
    println!("\n--- 2b. a node with NO top-level geometry is still indexed ---");

    let query = dwithin_in(
        WS_NESTED_ONLY,
        "hero.stage.geo",
        LON_HERO_STAGE,
        LAT,
        1_000.0,
    );
    ctx.expect_names(
        "a nested-only node is findable at its 3-level geometry",
        &query,
        &["s1"],
    )
    .await;

    // And index-backed, which is the stronger claim: a `SpatialDistanceScan`
    // requires a state record, and the state record is what the flat loop never
    // created.
    ctx.expect_index_backed("nested-only 'hero.stage.geo'", &query)
        .await;

    // Still a filter.
    ctx.expect_names(
        "the nested-only workspace discriminates by position",
        &dwithin_in(WS_NESTED_ONLY, "hero.stage.geo", LON_LOCATION, LAT, 1_000.0),
        &[],
    )
    .await;
}
