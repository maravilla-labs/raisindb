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

//! Sections 9–10 — tombstoning, the highest-risk half of nested indexing.
//!
//! A write emits index entries; only a **tombstone** removes them, and the
//! tombstone is derived from the OLD value's paths. Writer and tombstoner must
//! therefore agree byte-for-byte on the property path, which is why both go
//! through one walker: a format mismatch leaves entries nothing can ever shadow,
//! and the node keeps matching a position it has left.

use super::fixture::Ctx;
use super::{
    dwithin, point, LAT, LON_C0_PIN, LON_C0_STAGE, LON_C1_PIN, LON_HERO_PIN, LON_HERO_STAGE,
    LON_LOCATION, MAP_BLOCK, STAGE_BLOCK, V2_LAT, WS,
};

/// Where `hero.stage.geo` moves to — ~37 km east of where it was, and not a
/// position any other geometry occupies, so "the new position matches" cannot
/// pass for the wrong reason.
const MOVED_LON: f64 = 9.50;

/// A moved geometry three levels down stops matching where it was, and a shrunk
/// array drops its trailing element's path.
pub async fn a_moved_deep_geometry_stops_matching_where_it_was(ctx: &Ctx) {
    println!("\n--- 9. a moved 3-level geometry, and a shrunk array ---");

    ctx.expect_names(
        "precondition: 'hero.stage.geo' matches at its OLD position",
        &dwithin("hero.stage.geo", LON_HERO_STAGE, LAT, 1_000.0),
        &["v1"],
    )
    .await;
    ctx.expect_names(
        "precondition: 'content.1.pin' exists",
        &dwithin("content.1.pin", LON_C1_PIN, LAT, 1_000.0),
        &["v1"],
    )
    .await;

    // The stage moves; the array loses its second element. Everything else stays.
    ctx.run(&format!(
        "UPDATE '{WS}' SET properties = '{{\
           \"title\":\"Main\",\
           \"location\":{location},\
           \"hero\":{{\"element_type\":\"{MAP_BLOCK}\",\"pin\":{hero_pin},\
             \"stage\":{{\"element_type\":\"{STAGE_BLOCK}\",\"label\":\"main\",\
               \"geo\":{moved}}}}},\
           \"content\":[\
             {{\"element_type\":\"{MAP_BLOCK}\",\"pin\":{c0_pin},\
               \"stage\":{{\"element_type\":\"{STAGE_BLOCK}\",\"label\":\"c0\",\
                 \"geo\":{c0_stage}}}}}\
           ]\
         }}'::JSONB WHERE id = 'v1'",
        location = point(LON_LOCATION, LAT),
        hero_pin = point(LON_HERO_PIN, LAT),
        moved = point(MOVED_LON, LAT),
        c0_pin = point(LON_C0_PIN, LAT),
        c0_stage = point(LON_C0_STAGE, LAT),
    ))
    .await;

    ctx.expect_names(
        "after the move the OLD 3-level position no longer matches",
        &dwithin("hero.stage.geo", LON_HERO_STAGE, LAT, 1_000.0),
        &[],
    )
    .await;
    ctx.expect_names(
        "after the move the NEW 3-level position matches",
        &dwithin("hero.stage.geo", MOVED_LON, LAT, 1_000.0),
        &["v1"],
    )
    .await;
    ctx.expect_names(
        "the removed array element's path stops matching",
        &dwithin("content.1.pin", LON_C1_PIN, LAT, 1_000.0),
        &[],
    )
    .await;

    // Paths the update did not touch must be unaffected — a tombstoner that
    // over-reached would take these with it.
    for (path, lon) in [
        ("location", LON_LOCATION),
        ("hero.pin", LON_HERO_PIN),
        ("content.0.pin", LON_C0_PIN),
        ("content.0.stage.geo", LON_C0_STAGE),
    ] {
        ctx.expect_names(
            &format!("'{path}' is untouched by the update"),
            &dwithin(path, lon, LAT, 1_000.0),
            &["v1"],
        )
        .await;
    }

    // And the other node is entirely unaffected.
    ctx.expect_names(
        "v2's 3-level path is unaffected by v1's move",
        &dwithin("hero.stage.geo", LON_HERO_STAGE, V2_LAT, 1_000.0),
        &["v2"],
    )
    .await;
}

/// A deleted node stops matching on EVERY path, including the deep and array
/// ones. The delete path is a separate site from the update tombstoner, and it
/// was flat before this pass — a deleted node would have kept matching every
/// nested path forever.
pub async fn a_deleted_node_stops_matching_on_every_path(ctx: &Ctx) {
    println!("\n--- 10. a deleted node stops matching on every nested path ---");

    ctx.run(&format!("DELETE FROM '{WS}' WHERE id = 'v1'"))
        .await;

    for (path, lon) in [
        ("location", LON_LOCATION),
        ("hero.pin", LON_HERO_PIN),
        // Its position after section 9's move.
        ("hero.stage.geo", MOVED_LON),
        ("content.0.pin", LON_C0_PIN),
        ("content.0.stage.geo", LON_C0_STAGE),
    ] {
        ctx.expect_names(
            &format!("after DELETE, '{path}' no longer matches"),
            &dwithin(path, lon, LAT, 1_000.0),
            &[],
        )
        .await;
    }

    // The surviving node must still be findable on every one of its paths — a
    // delete that tombstoned by prefix rather than by node would take it too.
    for (path, lon) in [
        ("location", LON_LOCATION),
        ("hero.pin", LON_HERO_PIN),
        ("hero.stage.geo", LON_HERO_STAGE),
        ("content.0.pin", LON_C0_PIN),
        ("content.0.stage.geo", LON_C0_STAGE),
        ("content.1.pin", LON_C1_PIN),
    ] {
        ctx.expect_names(
            &format!("v2 still matches on '{path}' after v1 was deleted"),
            &dwithin(path, lon, V2_LAT, 1_000.0),
            &["v2"],
        )
        .await;
    }
}
