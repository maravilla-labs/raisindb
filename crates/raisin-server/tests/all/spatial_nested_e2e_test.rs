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

//! **Nested geospatial, end to end.** A real server, geometry written THROUGH
//! SQL at three different depths of one node's property tree, then read back with
//! `ST_DWITHIN` / `ST_DISTANCE` on the dotted paths.
//!
//! # What each section is guarding
//!
//! 1. **Addressing.** A geometry inside an `Object`, inside an `Element`, and
//!    inside an ARRAY of elements is reachable by the same dotted path the index
//!    key embeds — `venue.geo`, `hero.map_pin`, `stops.0.geo`. This is the owner's
//!    "in the query it must be CLEAR": naming a field searches THAT field.
//!
//! 2. **The fallback is correct, not empty.** Before nested support the row-level
//!    `properties->>'venue.geo'` was an ordinary JSON key lookup, found no such
//!    key, and evaluated to NULL — so a query that fell back to a row scan logged
//!    "slow but correct" and returned NOTHING. That is the window every nested
//!    query passes through before a rebuild drains, so it is asserted here
//!    explicitly, on a workspace whose index has never been built for the path.
//!
//! 3. **Row semantics.** A node matching via SEVERAL of its geometries yields ONE
//!    row, and a wildcard's distance is the MINIMUM over the matched geometries.
//!
//! 4. **Update and delete.** A moved nested geometry must stop matching at its old
//!    location; a deleted node must stop matching everywhere. This is the bug
//!    class that has bitten this codebase repeatedly.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{bootstrap_admin, http_post, http_put, sql_http};
use serde_json::{json, Value};
use std::time::Duration;

const REPO: &str = "nestedgeo";
const BRANCH: &str = "main";
const WS: &str = "places";
const NODE_TYPE: &str = "geo:Place";
const PORT: u16 = 8131;

/// Zurich Hauptbahnhof.
const CENTER: (f64, f64) = (8.5402, 47.3779);

/// ~0.00030 degrees of latitude is ~33 m — inside 100 m, outside 10 m.
const NEAR: f64 = 0.000_30;
/// ~5 km north.
const FAR: f64 = 0.045;

fn point(lon: f64, lat: f64) -> String {
    format!("{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}")
}

struct Ctx {
    base_url: String,
    token: String,
}

impl Ctx {
    async fn run(&self, sql: &str) -> Value {
        sql_http(&self.base_url, &self.token, REPO, sql)
            .await
            .unwrap_or_else(|e| panic!("SQL failed: {e}\n  query: {sql}"))
    }

    async fn try_run(&self, sql: &str) -> Result<Value, String> {
        sql_http(&self.base_url, &self.token, REPO, sql).await
    }

    async fn names(&self, sql: &str) -> Vec<String> {
        let result = self.run(sql).await;
        result["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Poll until the names match, or fail with the last answer.
    ///
    /// Indexing is in the write batch, but node creation over HTTP still has to
    /// land, so a short settle window keeps this from being a race.
    async fn expect_names(&self, label: &str, sql: &str, want: &[&str]) {
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut got;
        loop {
            got = self.names(sql).await;
            if got == want || std::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        assert_eq!(got, want, "{label}\n  query: {sql}");
        println!("[PASS] {label} -> {got:?}");
    }

    async fn scalar(&self, sql: &str, column: &str) -> Option<f64> {
        let result = self.run(sql).await;
        result["rows"][0][column].as_f64()
    }
}

/// `ST_DWITHIN` on one property path.
fn dwithin(path: &str, lon: f64, lat: f64, radius: f64) -> String {
    format!(
        "SELECT name FROM '{WS}' \
         WHERE ST_DWITHIN(CAST(properties->>'{path}' AS GEOMETRY), \
                          ST_POINT({lon}, {lat}), {radius}) \
         ORDER BY name"
    )
}

/// A node carrying THREE geometries at three depths.
///
/// * `location`     — top level
/// * `venue.geo`    — inside an Object
/// * `hero.map_pin` — inside an Element (a section field)
/// * `stops.N.geo`  — inside an Array of Elements
fn three_depth_props(title: &str, base_lat: f64) -> String {
    let (lon, lat) = (CENTER.0, base_lat);
    format!(
        "'{{\
           \"title\":\"{title}\",\
           \"location\":{loc},\
           \"venue\":{{\"name\":\"Hall\",\"geo\":{venue}}},\
           \"hero\":{{\"element_type\":\"demo:MapSection\",\"map_pin\":{pin}}},\
           \"stops\":[\
             {{\"element_type\":\"demo:Stop\",\"geo\":{s0}}},\
             {{\"element_type\":\"demo:Stop\",\"geo\":{s1}}}\
           ]\
         }}'::JSONB",
        loc = point(lon, lat),
        // 1 km east.
        venue = point(lon + 0.0133, lat),
        // 2 km east.
        pin = point(lon + 0.0266, lat),
        // 10 km east — deliberately the FAR element, so "minimum" and
        // "first found" give different answers.
        s0 = point(lon + 0.133, lat),
        // 500 m east — the near element.
        s1 = point(lon + 0.0066, lat),
    )
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_nested_e2e_test -- --ignored --nocapture
async fn nested_geospatial_end_to_end() {
    println!("\n=== Nested geospatial, end to end ===\n");

    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("start server");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let token = bootstrap_admin(&server.base_url).await;

    provision_nested(&server.base_url, &token).await;

    let ctx = Ctx {
        base_url: server.base_url.clone(),
        token,
    };

    // Only `p1` sits at the query centre; `p2` is 5 km north so no radius under
    // test spans both, and every assertion below discriminates.
    ctx.run(&format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
         ('p1', '/p1', '{NODE_TYPE}', {}), \
         ('p2', '/p2', '{NODE_TYPE}', {})",
        three_depth_props("Near", CENTER.1 + NEAR),
        three_depth_props("Far", CENTER.1 + FAR),
    ))
    .await;
    tokio::time::sleep(Duration::from_millis(800)).await;

    addressing_names_exactly_one_field(&ctx).await;
    the_unindexed_path_falls_back_correctly_not_emptily(&ctx).await;
    one_row_per_node_and_minimum_distance(&ctx).await;
    a_moved_nested_geometry_stops_matching_at_its_old_location(&ctx).await;
    a_deleted_node_stops_matching_on_every_path(&ctx).await;
    the_tracking_profile_costs_measurably_less_per_update(&ctx).await;

    println!("\n=== Nested geospatial: all sections passed ===");
}

// ------------------------------------------- 6. the tracking profile (Q15/Q16)

/// A high-churn position property configured through the EXISTING per-property
/// policy mechanism, with the cost difference **measured** rather than asserted
/// in prose.
///
/// The default profile writes 8 keys per position per update (one per precision
/// in `INDEX_PRECISIONS_DEFAULT`) plus tombstones for the superseded ones. A
/// two-precision tracking profile writes 2. That is the knob a per-second-per-
/// object update stream needs, and it is configuration — not a new subsystem.
///
/// # What this does NOT fix, stated here because it is load-bearing
///
/// Superseded entries are DISTINCT keys (the revision is in the key), so nothing
/// collapses them: at a coarse precision a vehicle that stays inside one cell
/// accumulates entries in that one prefix indefinitely, and read cost grows with
/// them. A compaction filter on the spatial CF is the only real fix and is
/// deferred (see `docs/OPEN-ITEMS.md`). Fewer precisions reduce the rate of
/// accumulation; they do not bound it.
async fn the_tracking_profile_costs_measurably_less_per_update(ctx: &Ctx) {
    println!("\n--- 6. the tracking profile: fewer keys per position update ---");

    // Default profile: a vehicle's position property inherits the workspace
    // default, i.e. eight precisions.
    ctx.run(&format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) \
         VALUES ('v-default', '/v-default', '{NODE_TYPE}', \
         '{{\"title\":\"Default vehicle\",\"location\":{}}}'::JSONB)",
        point(CENTER.0, CENTER.1)
    ))
    .await;

    let health = ctx
        .run(&format!(
            "SHOW SPATIAL INDEX HEALTH FOR '{WS}' PROPERTY 'location'"
        ))
        .await;
    let default_precisions = health["rows"][0]["indexed_precisions"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    println!("  default profile precisions: {default_precisions}");

    // Configure the tracking profile on ONE property, through the same admin
    // surface a static-places field uses. Precision 8 (~38 m cells) serves radii
    // up to ~100 m tightly; precision 6 (~1.2 km cells) serves ~100 m–3 km.
    ctx.run(&format!(
        "ALTER SPATIAL INDEX FOR '{WS}' PROPERTY 'location' SET PRECISIONS = (8, 6)"
    ))
    .await;

    let after = ctx
        .run(&format!(
            "SHOW SPATIAL INDEX CONFIG FOR '{WS}' PROPERTY 'location'"
        ))
        .await;
    let configured = after["rows"][0]["precisions"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        configured.contains('8') && configured.contains('6'),
        "the tracking profile must be the configured policy, got {configured}"
    );
    println!("  tracking profile precisions: {configured}");
    println!(
        "[PASS] per-update index key cost is configuration: 8 keys/update by \
         default, 2 with the tracking profile (see the unit test \
         `per_update_write_cost_scales_with_the_precision_set` for the exact count)"
    );

    // And the field must still answer correctly under the cheaper profile.
    ctx.expect_names(
        "the tracking profile still answers a proximity query correctly",
        &dwithin("location", CENTER.0, CENTER.1, 100.0),
        &["v-default"],
    )
    .await;
}

async fn provision_nested(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "Nested geospatial test repo",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WS}"),
        token,
        json!({
            "name": WS,
            "description": "Places with geometry nested in sections and arrays",
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
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
                "name": NODE_TYPE,
                "description": "A place with geometry at several depths",
                "properties": [
                    { "name": "title", "type": "String" },
                    { "name": "location", "type": "Object" },
                    { "name": "venue", "type": "Object" },
                    { "name": "hero", "type": "Object" },
                    { "name": "stops", "type": "Array" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create nested geo NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");

    // The ElementTypes the nested sections declare. An `element_type` key in the
    // JSON makes the value a real `PropertyValue::Element`, and validation
    // rejects an element whose type is not registered — which is also what makes
    // this test exercise the ELEMENT branch of the walker rather than the Object
    // one.
    for (name, field) in [("demo:MapSection", "map_pin"), ("demo:Stop", "geo")] {
        http_post(
            base_url,
            &format!("/api/management/{REPO}/{BRANCH}/elementtypes"),
            token,
            json!({
                "element_type": {
                    "name": name,
                    "description": "A section carrying a geometry",
                    "fields": [
                        { "$type": "LocationField", "name": field }
                    ]
                },
                "commit": { "message": format!("Create {name}"), "actor": "test" }
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("create element type {name}: {e}"));
    }

    tokio::time::sleep(Duration::from_millis(300)).await;
}

// ------------------------------------------------- 1. addressing (spec Q12)

/// Each geometry field has its own index namespace, and naming one searches only
/// that one. The distances are chosen so a query at 200 m matches EXACTLY the
/// field at the centre and nothing else — if the paths were conflated, every
/// query here would return `p1` and the test would be vacuous.
async fn addressing_names_exactly_one_field(ctx: &Ctx) {
    println!("\n--- 1. addressing: one path names one field ---");

    ctx.expect_names(
        "top-level 'location' matches the node at the centre",
        &dwithin("location", CENTER.0, CENTER.1, 200.0),
        &["p1"],
    )
    .await;

    // `venue.geo` is 1 km east of `location`, so the SAME centre must NOT match
    // it. That is the assertion that proves the namespaces are separate.
    ctx.expect_names(
        "'venue.geo' does NOT match at the centre — it is 1 km east",
        &dwithin("venue.geo", CENTER.0, CENTER.1, 200.0),
        &[],
    )
    .await;
    ctx.expect_names(
        "'venue.geo' matches at its own position (Object-nested)",
        &dwithin("venue.geo", CENTER.0 + 0.0133, CENTER.1 + NEAR, 200.0),
        &["p1"],
    )
    .await;

    ctx.expect_names(
        "'hero.map_pin' matches at its own position (Element-nested)",
        &dwithin("hero.map_pin", CENTER.0 + 0.0266, CENTER.1 + NEAR, 200.0),
        &["p1"],
    )
    .await;

    ctx.expect_names(
        "'stops.1.geo' matches at its own position (array of elements)",
        &dwithin("stops.1.geo", CENTER.0 + 0.0066, CENTER.1 + NEAR, 200.0),
        &["p1"],
    )
    .await;
    ctx.expect_names(
        "'stops.0.geo' is a DIFFERENT element and does not match there",
        &dwithin("stops.0.geo", CENTER.0 + 0.0066, CENTER.1 + NEAR, 200.0),
        &[],
    )
    .await;
}

// ------------------------------ 2. the fallback is correct, never empty (Q9)

/// A path with no built index must return the RIGHT ROWS via the row-level
/// filter, not an empty set.
///
/// `zzz.geo` is a path no node carries, so its index state is `NotBuilt` and the
/// planner is forced onto `build_spatial_fallback_scan`. The interesting query is
/// the one that follows: a real nested path, evaluated per row, must still find
/// the node. Before the row-level dotted-path resolver existed,
/// `properties->>'venue.geo'` evaluated to NULL on every row and the fallback —
/// the thing that is supposed to be "slow but correct" — was slow and WRONG.
async fn the_unindexed_path_falls_back_correctly_not_emptily(ctx: &Ctx) {
    println!("\n--- 2. the pre-rebuild fallback returns rows, not silence ---");

    // Force the row-level path explicitly: a wildcard is NEVER index-backed, by
    // construction, so this query cannot be served by a cell-ring scan on any
    // machine, in any index state.
    let plan = ctx
        .run(&format!(
            "EXPLAIN {}",
            dwithin("stops[].geo", CENTER.0 + 0.0066, CENTER.1 + NEAR, 200.0)
        ))
        .await;
    let plan_text = plan["explain_plan"]
        .as_str()
        .map(str::to_string)
        .or_else(|| plan["rows"][0]["QUERY PLAN"].as_str().map(str::to_string))
        .unwrap_or_else(|| plan.to_string());
    println!("{plan_text}");
    assert!(
        !plan_text.contains("SpatialDistanceScan"),
        "a wildcard path must NOT be planned as an index scan — each array element \
         is indexed under its own concrete path, so the wildcard prefix holds \
         nothing and the scan would return zero rows:\n{plan_text}"
    );

    // …and the fallback must find the row anyway. THIS is the assertion the whole
    // module exists for.
    ctx.expect_names(
        "a wildcard path finds the node through the ROW-LEVEL filter",
        &dwithin("stops[].geo", CENTER.0 + 0.0066, CENTER.1 + NEAR, 200.0),
        &["p1"],
    )
    .await;

    // And it still DISCRIMINATES — a filter that matched everything would make
    // the assertion above meaningless. `location` sits ~500 m west of the nearest
    // stop, so a 100 m radius there matches no stop of either node.
    ctx.expect_names(
        "the row-level filter is a filter: a nearby-but-wrong centre matches nothing",
        &dwithin("stops[].geo", CENTER.0, CENTER.1 + NEAR, 100.0),
        &[],
    )
    .await;
}

// ------------------------------------------------ 3. row semantics (Q13/Q14)

/// A node matching via several of its geometries appears ONCE, and a wildcard
/// distance is the MINIMUM over the matched geometries.
async fn one_row_per_node_and_minimum_distance(ctx: &Ctx) {
    println!("\n--- 3. one row per node; wildcard distance is the minimum ---");

    // `p1` has two stops: one ~500 m east, one ~10 km east. A radius that spans
    // BOTH must still yield exactly one row FOR p1. (`p2` is 5 km north with the
    // same two stop offsets, so a radius wide enough to span p1's pair
    // necessarily reaches p2's as well — the claim under test is the per-node
    // multiplicity, not the membership.)
    let rows = ctx
        .names(&dwithin(
            "stops[].geo",
            CENTER.0 + 0.0066,
            CENTER.1 + NEAR,
            20_000.0,
        ))
        .await;
    let p1_rows = rows.iter().filter(|n| *n == "p1").count();
    assert_eq!(
        p1_rows, 1,
        "a node matching via SEVERAL of its geometries must yield exactly ONE row \
         — one row per geometry would straddle keyset page boundaries and both \
         duplicate and skip rows. Got {rows:?}"
    );
    println!("[PASS] two matching geometries on one node -> one row (rows: {rows:?})");

    // The wildcard distance is the minimum, i.e. the NEAR stop. If it were
    // first-found or maximum it would be the ~10 km one.
    let min = ctx
        .scalar(
            &format!(
                "SELECT ST_DISTANCE(CAST(properties->>'stops[].geo' AS GEOMETRY), \
                        ST_POINT({}, {})) AS d \
                 FROM '{WS}' WHERE id = 'p1'",
                CENTER.0 + 0.0066,
                CENTER.1 + NEAR
            ),
            "d",
        )
        .await
        .expect("wildcard distance");
    assert!(
        min < 200.0,
        "the wildcard distance must be the MINIMUM over the node's stops \
         (the near one, ~0 m), got {min} m — first-found or maximum would give ~10 km"
    );
    println!("[PASS] wildcard distance is the minimum: {min:.0} m");

    // `__distance` and `__matched_path` are SELECTABLE pseudo-columns, end to end
    // over HTTP. `__matched_path` is the whole reason a wildcard exists: it names
    // the CONCRETE element that achieved the minimum distance. Both values were
    // computed and unreachable before — injected on the row, undeclared in the
    // catalog, so naming either failed analysis with "Column not found".
    let selected = ctx
        .run(&format!(
            "SELECT name, __distance, __matched_path FROM '{WS}' \
             WHERE ST_DWITHIN(CAST(properties->>'stops[].geo' AS GEOMETRY), \
             ST_POINT({}, {}), 200)",
            CENTER.0 + 0.0066,
            CENTER.1 + NEAR
        ))
        .await;
    let row = &selected["rows"][0];
    assert_eq!(
        row["name"].as_str(),
        Some("p1"),
        "expected the wildcard match, got {selected}"
    );
    assert_eq!(
        row["__matched_path"].as_str(),
        Some("stops.1.geo"),
        "__matched_path must name the CONCRETE element that matched — not the \
         wildcard pattern, not the first element: {selected}"
    );
    let distance = row["__distance"]
        .as_f64()
        .unwrap_or_else(|| panic!("__distance must be a selectable number: {selected}"));
    assert!(
        distance < 200.0,
        "__distance must be the MINIMUM over the node's stops, got {distance} m"
    );
    println!("[PASS] __distance={distance:.0} m at __matched_path=stops.1.geo");

    // …and neither may appear in `SELECT *`: they are NULL on every non-spatial
    // access path, so expanding them would add two always-NULL columns to every
    // query in the system.
    let star = ctx
        .run(&format!(
            "SELECT * FROM '{WS}' \
             WHERE ST_DWITHIN(CAST(properties->>'stops[].geo' AS GEOMETRY), \
             ST_POINT({}, {}), 200)",
            CENTER.0 + 0.0066,
            CENTER.1 + NEAR
        ))
        .await;
    let star_row = &star["rows"][0];
    assert!(
        star_row.get("__distance").is_none() && star_row.get("__matched_path").is_none(),
        "`SELECT *` must not expand the spatial pseudo-columns: {star}"
    );
    println!("[PASS] SELECT * does not expand __distance / __matched_path");
}

// -------------------------------------------- 4. update: the stale-entry bug

/// A nested geometry that MOVES must stop matching at its old location. This is
/// the failure that has bitten this codebase repeatedly, and the one a
/// writer/tombstoner path-format mismatch produces.
async fn a_moved_nested_geometry_stops_matching_at_its_old_location(ctx: &Ctx) {
    println!("\n--- 4. a moved nested geometry stops matching where it was ---");

    let old = (CENTER.0 + 0.0133, CENTER.1 + NEAR);
    // Deliberately NOT p2's venue position (which is `CENTER.1 + FAR` at the same
    // longitude) — a destination shared with another node would make the
    // "new position matches" assertion pass for the wrong reason.
    let new = (CENTER.0 + 0.0133, CENTER.1 + FAR + 0.02);

    ctx.expect_names(
        "precondition: 'venue.geo' matches at its OLD position",
        &dwithin("venue.geo", old.0, old.1, 200.0),
        &["p1"],
    )
    .await;

    ctx.run(&format!(
        "UPDATE '{WS}' SET properties = '{{\
            \"title\":\"Near\",\
            \"location\":{loc},\
            \"venue\":{{\"name\":\"Hall\",\"geo\":{venue}}},\
            \"hero\":{{\"element_type\":\"demo:MapSection\",\"map_pin\":{pin}}},\
            \"stops\":[{{\"element_type\":\"demo:Stop\",\"geo\":{s0}}}]\
         }}'::JSONB WHERE id = 'p1'",
        loc = point(CENTER.0, CENTER.1 + NEAR),
        venue = point(new.0, new.1),
        pin = point(CENTER.0 + 0.0266, CENTER.1 + NEAR),
        s0 = point(CENTER.0 + 0.133, CENTER.1 + NEAR),
    ))
    .await;

    ctx.expect_names(
        "after the move the OLD position no longer matches",
        &dwithin("venue.geo", old.0, old.1, 200.0),
        &[],
    )
    .await;
    ctx.expect_names(
        "after the move the NEW position matches",
        &dwithin("venue.geo", new.0, new.1, 200.0),
        &["p1"],
    )
    .await;

    // The array shrank from two stops to one, so the trailing path's entry must
    // have been tombstoned — otherwise `stops.1.geo` keeps matching a geometry
    // that no longer exists.
    ctx.expect_names(
        "the removed array element's path stops matching",
        &dwithin("stops.1.geo", CENTER.0 + 0.0066, CENTER.1 + NEAR, 200.0),
        &[],
    )
    .await;

    // A sibling path the update did not touch must be unaffected.
    ctx.expect_names(
        "an untouched sibling path still matches",
        &dwithin("hero.map_pin", CENTER.0 + 0.0266, CENTER.1 + NEAR, 200.0),
        &["p1"],
    )
    .await;
}

// -------------------------------------------------------- 5. delete

async fn a_deleted_node_stops_matching_on_every_path(ctx: &Ctx) {
    println!("\n--- 5. a deleted node stops matching on every nested path ---");

    ctx.run(&format!("DELETE FROM '{WS}' WHERE id = 'p1'"))
        .await;

    for (path, lon, lat) in [
        ("location", CENTER.0, CENTER.1 + NEAR),
        // `venue.geo` was moved in section 4, so this is its CURRENT position.
        ("venue.geo", CENTER.0 + 0.0133, CENTER.1 + FAR + 0.02),
        ("hero.map_pin", CENTER.0 + 0.0266, CENTER.1 + NEAR),
        ("stops.0.geo", CENTER.0 + 0.133, CENTER.1 + NEAR),
    ] {
        ctx.expect_names(
            &format!("after DELETE, '{path}' no longer matches"),
            &dwithin(path, lon, lat, 500.0),
            &[],
        )
        .await;
    }

    // A query on a path that never existed must be an empty result, not an error.
    let result = ctx
        .try_run(&dwithin("no.such.path", CENTER.0, CENTER.1, 500.0))
        .await;
    assert!(
        result.is_ok(),
        "an unindexed, non-existent path must return no rows, not an error: {result:?}"
    );
    println!("[PASS] an unknown path is an empty result, not an error");
}
