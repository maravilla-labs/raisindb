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

//! The modelled shape, declared the way the shipped example packages declare it.
//!
//! * **ElementType** — `fields:` entries tagged `$type`, e.g. `$type: LocationField`
//!   (`examples/events/package/elementtypes/hero-block.yaml`).
//! * **Archetype** — `base_node_type` plus `fields:`, including
//!   `$type: SectionField` with `allowed_element_types`
//!   (`examples/events/package/archetypes/venue-page.yaml`).
//! * **NodeType** — `properties:` with a coarse storage `type`
//!   (`examples/events/package/nodetypes/event.yaml`).
//!
//! `geo:StageBlock` nested inside `geo:MapBlock` via an `ElementField` is what
//! produces a geometry THREE levels down (`hero.stage.geo`) and FOUR levels down
//! inside an array (`content.0.stage.geo`).

use super::fixture::Ctx;
use super::{
    point, ARCHETYPE, BRANCH, LAT, LON_C0_PIN, LON_C0_STAGE, LON_C1_PIN, LON_HERO_PIN,
    LON_HERO_STAGE, LON_LOCATION, MAP_BLOCK, NODE_TYPE, REPO, STAGE_BLOCK, V2_LAT, WS,
    WS_NESTED_ONLY,
};
use crate::helpers::sql_geo::{http_post, http_put};
use serde_json::json;
use std::time::Duration;

pub async fn provision(ctx: &Ctx) {
    let base = ctx.base_url.as_str();
    let token = ctx.token.as_str();

    http_post(
        base,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "Nested geospatial on a modelled archetype shape",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base,
        &format!("/api/workspaces/{REPO}/{WS}"),
        token,
        json!({
            "name": WS,
            "description": "Venues whose sections carry geometry",
            // `raisin:Folder` must be allowed: creating a workspace materialises a
            // root folder node, so a workspace permitting only its own type is
            // rejected at creation time.
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create workspace");

    // The innermost element type: a stage with its own coordinates.
    element_type(
        ctx,
        STAGE_BLOCK,
        json!([
            { "$type": "TextField", "name": "label", "title": "Label" },
            { "$type": "LocationField", "name": "geo", "title": "Stage position" }
        ]),
    )
    .await;

    // A section block carrying BOTH its own geometry and a nested element that
    // carries another one. This is the shape that makes `hero.stage.geo` real.
    element_type(
        ctx,
        MAP_BLOCK,
        json!([
            { "$type": "LocationField", "name": "pin", "title": "Map pin" },
            {
                "$type": "ElementField",
                "name": "stage",
                "title": "Stage",
                "element_type": STAGE_BLOCK
            }
        ]),
    )
    .await;

    // A second workspace whose nodes carry NO top-level geometry at all.
    http_put(
        base,
        &format!("/api/workspaces/{REPO}/{WS_NESTED_ONLY}"),
        token,
        json!({
            "name": WS_NESTED_ONLY,
            "description": "Sections whose ONLY geometry is nested",
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create nested-only workspace");

    http_post(
        base,
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        token,
        json!({
            "node_type": {
                "name": NODE_TYPE,
                "title": "Venue",
                "description": "A venue whose sections carry geometry",
                "properties": [
                    { "name": "title", "title": "Title", "type": "String" },
                    { "name": "location", "title": "Location", "type": "Object" },
                    { "name": "hero", "title": "Hero", "type": "Object" },
                    { "name": "content", "title": "Content", "type": "Array" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create geo:Venue", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");

    // The archetype: an editing shape over `geo:Venue` whose `content` is a real
    // SectionField admitting `geo:MapBlock`.
    http_post(
        base,
        &format!("/api/management/{REPO}/{BRANCH}/archetypes"),
        token,
        json!({
            "archetype": {
                "name": ARCHETYPE,
                "title": "Venue Page",
                "base_node_type": NODE_TYPE,
                "fields": [
                    { "$type": "TextField", "name": "title", "title": "Title" },
                    { "$type": "LocationField", "name": "location", "title": "Location" },
                    {
                        "$type": "ElementField",
                        "name": "hero",
                        "title": "Hero",
                        "element_type": MAP_BLOCK
                    },
                    {
                        "$type": "SectionField",
                        "name": "content",
                        "title": "Content",
                        "allowed_element_types": [MAP_BLOCK]
                    }
                ]
            },
            "commit": { "message": "Create geo:VenuePage", "actor": "test" }
        }),
    )
    .await
    .expect("create archetype");

    tokio::time::sleep(Duration::from_millis(400)).await;
    println!("[OK] provisioned {ARCHETYPE} over {NODE_TYPE} with {MAP_BLOCK}/{STAGE_BLOCK}");
}

async fn element_type(ctx: &Ctx, name: &str, fields: serde_json::Value) {
    http_post(
        &ctx.base_url,
        &format!("/api/management/{REPO}/{BRANCH}/elementtypes"),
        &ctx.token,
        json!({
            "element_type": {
                "name": name,
                "title": name,
                "description": "A section block carrying geometry",
                "fields": fields
            },
            "commit": { "message": format!("Create {name}"), "actor": "test" }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("create element type {name}: {e}"));
}

/// One venue's whole property tree, at a given latitude.
///
/// An object carrying an `element_type` key deserialises to a real
/// `PropertyValue::Element`, so `hero` and each `content` entry exercise the
/// Element branch of the walker rather than the plain Object branch — and
/// `hero.stage` exercises an Element nested inside an Element.
fn venue_properties(title: &str, lat: f64) -> String {
    format!(
        "'{{\
           \"title\":\"{title}\",\
           \"location\":{location},\
           \"hero\":{{\"element_type\":\"{MAP_BLOCK}\",\"pin\":{hero_pin},\
             \"stage\":{{\"element_type\":\"{STAGE_BLOCK}\",\"label\":\"main\",\
               \"geo\":{hero_stage}}}}},\
           \"content\":[\
             {{\"element_type\":\"{MAP_BLOCK}\",\"pin\":{c0_pin},\
               \"stage\":{{\"element_type\":\"{STAGE_BLOCK}\",\"label\":\"c0\",\
                 \"geo\":{c0_stage}}}}},\
             {{\"element_type\":\"{MAP_BLOCK}\",\"pin\":{c1_pin}}}\
           ]\
         }}'::JSONB",
        location = point(LON_LOCATION, lat),
        hero_pin = point(LON_HERO_PIN, lat),
        hero_stage = point(LON_HERO_STAGE, lat),
        c0_pin = point(LON_C0_PIN, lat),
        c0_stage = point(LON_C0_STAGE, lat),
        c1_pin = point(LON_C1_PIN, lat),
    )
}

/// Two venues written THROUGH SQL, each carrying all six geometries.
///
/// `v2` sits ~55 km north with the same longitudes, so every assertion below
/// discriminates between the two nodes as well as between the six paths.
pub async fn seed(ctx: &Ctx) {
    ctx.run(&format!(
        "INSERT INTO '{WS}' (id, path, node_type, archetype, properties) VALUES \
         ('v1', '/v1', '{NODE_TYPE}', '{ARCHETYPE}', {v1}), \
         ('v2', '/v2', '{NODE_TYPE}', '{ARCHETYPE}', {v2})",
        v1 = venue_properties("Main", LAT),
        v2 = venue_properties("North", V2_LAT),
    ))
    .await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    println!("[OK] seeded v1 and v2, six geometry paths each");

    seed_nested_only(ctx).await;
}

/// One node in `WS_NESTED_ONLY` whose ONLY geometry is three levels down.
///
/// No `location`, no `content` — nothing at the top level is a geometry. The
/// write path's "does this node carry geometry?" guard used to answer by scanning
/// `node.properties` flat, so this node was skipped entirely: no entries, no
/// state record, nothing.
async fn seed_nested_only(ctx: &Ctx) {
    ctx.run(&format!(
        "INSERT INTO '{WS_NESTED_ONLY}' (id, path, node_type, archetype, properties) VALUES \
         ('s1', '/s1', '{NODE_TYPE}', '{ARCHETYPE}', \
          '{{\"title\":\"Nested only\",\
             \"hero\":{{\"element_type\":\"{MAP_BLOCK}\",\
               \"stage\":{{\"element_type\":\"{STAGE_BLOCK}\",\"label\":\"only\",\
                 \"geo\":{geo}}}}}}}'::JSONB)",
        geo = point(LON_HERO_STAGE, LAT),
    ))
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("[OK] seeded s1 in '{WS_NESTED_ONLY}': one geometry, three levels down, none at top");
}
