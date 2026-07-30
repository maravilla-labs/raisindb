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

//! **Nested geospatial on a REAL modelled shape, end to end.**
//!
//! ```bash
//! cargo test -p raisin-server --test all spatial_nested_shape_e2e_test -- --ignored --nocapture
//! ```
//!
//! The sibling `spatial_nested_e2e_test` proves the walker on a hand-built
//! property tree. This module proves the same thing on the shape the product
//! actually models content with — an **Archetype** whose `SectionField` admits
//! **ElementTypes**, one of which carries a `LocationField` and an `ElementField`
//! pointing at a second ElementType that carries another `LocationField`. That is
//! the owner's case verbatim: *"we have many times nested properties with section
//! field node types and element types which can contain a geospatial field."*
//!
//! The declarations follow `examples/events/package/` (archetype `fields:` with
//! `$type: SectionField` + `allowed_element_types`, element types with
//! `$type: LocationField`); nothing here is an invented shape.
//!
//! # The six geometry paths one node carries
//!
//! | path | depth | container |
//! |---|---|---|
//! | `location` | 1 | top level |
//! | `hero.pin` | 2 | Element |
//! | `hero.stage.geo` | 3 | Element in an Element |
//! | `content.0.pin` | 3 | Element in an Array |
//! | `content.0.stage.geo` | 4 | Element in an Element in an Array |
//! | `content.1.pin` | 3 | a second array Element |
//!
//! Every one sits ~7.5 km from its neighbour, so a 1 km radius isolates exactly
//! one of them. If any two paths were conflated — the failure the whole pass is
//! about — the "and NOT the others" assertions would fail immediately.
//!
//! # What each section guards
//!
//! 1. `depth` — every path is findable **and index-backed** (`EXPLAIN` must show
//!    `SpatialDistanceScan`; correct rows alone cannot tell an index from a full
//!    scan, and a full scan would hide exactly the bug being fixed).
//! 2. `semantics` — one row per node, wildcard distance is the minimum,
//!    `ORDER BY ST_DISTANCE … LIMIT k` stays correct with several matches, and an
//!    unindexed path is slow-but-correct rather than silently empty.
//! 3. `migration` — per-property precision policy applies to a **nested** path,
//!    and a `POST …/spatial/rebuild` re-derives every nested entry, which is the
//!    migration story for data written before this pass.
//! 4. `mutation` — a moved deep geometry stops matching where it was, a shrunk
//!    array drops its trailing path, and a delete clears every path.
//!
//! Sections run in one server because the fixture (repo + archetype + element
//! types + indexed nodes) costs ~30 s to build and every later section asserts
//! against an index an earlier one proved correct.

mod depth;
mod fixture;
mod migration;
mod mutation;
mod schema;
mod semantics;

use crate::helpers::multi_node::{ServerConfig, ServerHandle};
use fixture::Ctx;
use std::time::Duration;

pub const TENANT: &str = "default";
pub const REPO: &str = "nestedshape";
pub const BRANCH: &str = "main";
pub const WS: &str = "venues";
/// A workspace where NO node carries a top-level geometry — every geometry is
/// nested. The write path used to decide whether to index at all by scanning
/// `node.properties` flat, so a workspace shaped like this was not partially
/// indexed, it was not indexed AT ALL.
pub const WS_NESTED_ONLY: &str = "sections";
pub const NODE_TYPE: &str = "geo:Venue";
pub const ARCHETYPE: &str = "geo:VenuePage";
pub const MAP_BLOCK: &str = "geo:MapBlock";
pub const STAGE_BLOCK: &str = "geo:StageBlock";

/// Not shared with any other module in this binary.
const PORT: u16 = 8261;

/// Zurich-ish. `V2_LAT` is ~55 km north, far enough that no radius under test
/// spans both nodes, so every row-set assertion discriminates between them.
pub const LAT: f64 = 47.38;
pub const V2_LAT: f64 = 47.88;

/// The six longitudes, 0.10° apart — ~7.5 km at this latitude.
pub const LON_LOCATION: f64 = 8.50;
pub const LON_HERO_PIN: f64 = 8.60;
pub const LON_HERO_STAGE: f64 = 8.70;
pub const LON_C0_PIN: f64 = 8.80;
pub const LON_C0_STAGE: f64 = 8.90;
pub const LON_C1_PIN: f64 = 9.00;

/// Every geometry path a fixture node carries, with the longitude it sits at.
///
/// The order is the order `walk_geometries` emits (sorted by path), which is also
/// the order the assertions read best in.
pub const PATHS: [(&str, f64); 6] = [
    ("content.0.pin", LON_C0_PIN),
    ("content.0.stage.geo", LON_C0_STAGE),
    ("content.1.pin", LON_C1_PIN),
    ("hero.pin", LON_HERO_PIN),
    ("hero.stage.geo", LON_HERO_STAGE),
    ("location", LON_LOCATION),
];

/// A GeoJSON point literal.
pub fn point(lon: f64, lat: f64) -> String {
    format!("{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}")
}

/// `ST_DWITHIN` on one property path, ordered so the row set is comparable.
pub fn dwithin(path: &str, lon: f64, lat: f64, radius: f64) -> String {
    dwithin_in(WS, path, lon, lat, radius)
}

/// [`dwithin`] against a named workspace.
pub fn dwithin_in(ws: &str, path: &str, lon: f64, lat: f64, radius: f64) -> String {
    format!(
        "SELECT name FROM '{ws}' \
         WHERE ST_DWITHIN(CAST(properties->>'{path}' AS GEOMETRY), \
                          ST_POINT({lon}, {lat}), {radius}) \
         ORDER BY name"
    )
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_nested_shape_e2e_test -- --ignored --nocapture
async fn nested_geospatial_on_a_modelled_shape() {
    println!("\n=== Nested geospatial on a modelled Archetype/ElementType shape ===\n");

    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let ctx = Ctx::bootstrap(&server.base_url).await;
    schema::provision(&ctx).await;
    schema::seed(&ctx).await;

    depth::every_nested_path_is_index_backed_and_independent(&ctx).await;
    depth::top_level_and_nested_on_one_node_are_independent(&ctx).await;
    depth::a_node_whose_only_geometry_is_nested_is_indexed(&ctx).await;
    semantics::one_row_per_node_and_minimum_distance(&ctx).await;
    semantics::distance_ordering_with_several_matching_geometries(&ctx).await;
    semantics::an_unindexed_path_is_slow_but_never_empty(&ctx).await;
    migration::precision_policy_applies_to_a_nested_path(&ctx).await;
    migration::a_wildcard_policy_key_configures_every_array_element(&ctx).await;
    migration::a_full_rebuild_re_derives_every_nested_entry(&ctx).await;
    mutation::a_moved_deep_geometry_stops_matching_where_it_was(&ctx).await;
    mutation::a_deleted_node_stops_matching_on_every_path(&ctx).await;

    println!("\n=== Nested geospatial (modelled shape): all sections passed ===");
}
