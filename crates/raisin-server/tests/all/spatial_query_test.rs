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

//! The spatial **query surface**, end to end against a real server.
//!
//! Every assertion here goes over HTTP `POST /api/sql/{repo}` to a server process
//! started by `helpers::multi_node::ServerHandle`, against data written through
//! the public write paths. Unit tests are not accepted as proof for this area:
//! the defects being guarded against were all *composition* failures — a planner
//! decision that was individually reasonable and wrong once combined with the
//! catalog, the executor or a second predicate.
//!
//! # What each test pins down
//!
//! * [`index_eligible_predicate_shapes_return_the_truth_and_use_the_index`] — the
//!   widened predicate set. Before this pass exactly ONE spelling reached the
//!   index (`ST_DWITHIN(<source>, ST_POINT(<lit>,<lit>), <lit double>)`);
//!   a reversed argument order, an integer radius, a `ST_GEOMFROMGEOJSON` centre
//!   or `ST_DISTANCE(...) < r` all silently fell back to a full scan. Each shape
//!   is asserted twice: on its **rows** (correctness) and on its **EXPLAIN plan**
//!   (that the access path is the one intended). Asserting only rows would pass
//!   for a full scan, and asserting only the plan would pass for a scan that
//!   returns the wrong rows.
//! * [`order_by_st_distance_limit_k_is_ordered_and_index_backed`] — `SpatialKnnScan`
//!   was dead code: the plan variant, the executor and the storage method all
//!   existed and no planner site ever built one. Nearest-neighbour ordering is
//!   the canonical geospatial query and it had no test at all.
//! * [`an_unbounded_distance_sort_falls_back_and_is_still_ordered`] — the same
//!   ordering through the generic sort path, which is where a `DESC` or
//!   `LIMIT`-less distance sort has to go because neither has a bounded access
//!   path. Separated from the k-NN test so a defect in the generic path cannot
//!   mask the index-backed one.
//! * [`spatial_composes_with_node_type_property_and_hierarchy`] — the planner
//!   picks ONE access path and every other predicate must survive as a residual
//!   filter. Dropping one side is the class of bug CLAUDE.md records for
//!   `REFERENCES(...) AND DESCENDANT_OF(...)`, which silently returned zero rows.
//! * [`an_unbuilt_spatial_index_falls_back_instead_of_returning_nothing`] — the
//!   silent-empty trap. `has_spatial_index()` used to be a hardcoded `true` (a
//!   claim about the column family existing, not about anything having been
//!   indexed) and the planner stripped `ST_DWITHIN` from the residual filter on
//!   the strength of it, so an unpopulated index returned ZERO ROWS with no
//!   error and no fallback.
//! * [`st_functions_round_trip_over_stored_geometry`] — the ST_\* engine over real
//!   stored node data rather than literal-vs-literal expressions, including
//!   `Multi*` and `GeometryCollection`, which no predicate or measurement
//!   function accepted as input before the conversion layer landed.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const REPO: &str = "spatial_query";
const BRANCH: &str = "main";
/// Point geometries in a hierarchy, with a `floor` discriminator.
const PLACES: &str = "places";
/// One node per geometry type, for the ST_\* round trip.
const SHAPES: &str = "shapes";
/// Geometry stored as a JSON **string**, so it is never spatially indexed —
/// the fixture for the fallback test.
const RAW: &str = "raw";

/// Zurich Hauptbahnhof. Chosen because `(8.5402, 47.3782)` reversed is not a
/// plausible coordinate pair, so an axis-order regression cannot pass silently.
const CENTER_LON: f64 = 8.5402;
const CENTER_LAT: f64 = 47.3782;

// At 47.3782°N one degree of latitude is ~111_132 m, so the offsets below put
// the fixture at ~37 m, ~56 m, ~244 m, ~25 km and ~222 km from the centre.
const D_37M: f64 = 0.000_33;
const D_56M: f64 = 0.000_50;
const D_244M: f64 = 0.002_2;
const D_25KM: f64 = 0.225;
const D_222KM: f64 = 2.0;

// --- HTTP plumbing -----------------------------------------------------------

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

/// A running server with an authenticated admin token and the fixture in place.
struct Fixture {
    server: ServerHandle,
    token: String,
}

impl Fixture {
    async fn sql(&self, query: &str) -> Result<Value, String> {
        self.sql_params(query, vec![]).await
    }

    async fn sql_params(&self, query: &str, params: Vec<Value>) -> Result<Value, String> {
        http_post(
            &self.server.base_url,
            &format!("/api/sql/{REPO}"),
            &self.token,
            json!({ "sql": query, "params": params }),
        )
        .await
    }

    /// The `name` column of every row, sorted — the shape almost every assertion
    /// here wants, because set equality is what "returns the truth" means.
    async fn names(&self, query: &str) -> Result<Vec<String>, String> {
        let result = self.sql(query).await?;
        let mut names: Vec<String> = result["rows"]
            .as_array()
            .ok_or_else(|| format!("no rows array in {result}"))?
            .iter()
            .map(|r| r["name"].as_str().unwrap_or("<no name>").to_string())
            .collect();
        names.sort();
        Ok(names)
    }

    /// The `name` column in the order the server returned it. Distinct from
    /// [`Self::names`] on purpose: an ordering assertion that sorts its own input
    /// asserts nothing.
    async fn names_in_order(&self, query: &str) -> Result<Vec<String>, String> {
        let result = self.sql(query).await?;
        Ok(result["rows"]
            .as_array()
            .ok_or_else(|| format!("no rows array in {result}"))?
            .iter()
            .map(|r| r["name"].as_str().unwrap_or("<no name>").to_string())
            .collect())
    }

    /// The physical plan text for a query, from `EXPLAIN`.
    async fn explain(&self, query: &str) -> String {
        let result = self
            .sql(&format!("EXPLAIN {query}"))
            .await
            .unwrap_or_else(|e| panic!("EXPLAIN failed\n  {query}\n  {e}"));
        result["rows"][0]["QUERY PLAN"]
            .as_str()
            .unwrap_or_else(|| panic!("EXPLAIN returned no QUERY PLAN column: {result}"))
            .to_string()
    }

    /// A single scalar from the first row.
    async fn scalar(&self, query: &str, column: &str) -> Value {
        let result = self
            .sql(query)
            .await
            .unwrap_or_else(|e| panic!("SQL failed\n  {query}\n  {e}"));
        result["rows"][0][column].clone()
    }
}

// --- fixture construction ----------------------------------------------------

/// Start a server, clear the initial-password flag, and build the three
/// workspaces the tests share.
async fn setup(port: u16) -> Fixture {
    let server = ServerHandle::start(ServerConfig::new(port))
        .await
        .expect("failed to start server");

    // The admin user is created asynchronously after the listener is up.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("failed to authenticate");

    let client = Client::new();
    let profile: Value = client
        .get(format!("{}/api/raisindb/me", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .expect("me request")
        .json()
        .await
        .expect("me json");
    let user_id = profile["user_id"].as_str().expect("user_id");
    client
        .put(format!(
            "{}/api/raisindb/sys/default/users/{}",
            server.base_url, user_id
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await
        .expect("clear must_change_password");

    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("failed to re-authenticate");

    let fixture = Fixture { server, token };

    http_post(
        &fixture.server.base_url,
        "/api/repositories",
        &fixture.token,
        json!({
            "repo_id": REPO,
            "description": "spatial query surface",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    create_workspace(&fixture, PLACES, &["q:Shop", "q:Kiosk"]).await;
    create_workspace(&fixture, SHAPES, &["q:Shape"]).await;
    create_workspace(&fixture, RAW, &["q:Raw"]).await;

    // `location` / `geom` are declared `Geometry`, which is the first-class
    // declaration: indexing itself is driven by the runtime PropertyValue type,
    // but declaring the type is what makes a malformed value a loud write error
    // rather than an unindexed `Object`.
    create_nodetype(
        &fixture,
        "q:Shop",
        "location",
        "Geometry",
        &["q:Shop", "q:Kiosk"],
    )
    .await;
    create_nodetype(&fixture, "q:Kiosk", "location", "Geometry", &[]).await;
    create_nodetype(&fixture, "q:Shape", "geom", "Geometry", &[]).await;
    // Deliberately String: a JSON string is not a `PropertyValue::Geometry`, so
    // nothing indexes it and the query must fall back rather than return nothing.
    create_nodetype(&fixture, "q:Raw", "location", "String", &[]).await;

    fixture
}

async fn create_workspace(fixture: &Fixture, name: &str, node_types: &[&str]) {
    // `raisin:Folder` must be allowed as a root type: creating a workspace
    // materialises its `/` root node as one, and the workspace validator rejects
    // its own root otherwise.
    let mut node_types: Vec<&str> = node_types.to_vec();
    node_types.push("raisin:Folder");
    http_put(
        &fixture.server.base_url,
        &format!("/api/workspaces/{REPO}/{name}"),
        &fixture.token,
        json!({
            "name": name,
            "description": format!("{name} workspace"),
            "allowed_node_types": node_types,
            "allowed_root_node_types": node_types,
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("create workspace {name}: {e}"));
}

async fn create_nodetype(
    fixture: &Fixture,
    name: &str,
    geometry_property: &str,
    geometry_type: &str,
    allowed_children: &[&str],
) {
    http_post(
        &fixture.server.base_url,
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        &fixture.token,
        json!({
            "node_type": {
                "name": name,
                "description": format!("{name} for the spatial query surface tests"),
                "properties": [
                    { "name": "title", "type": "String" },
                    { "name": "floor", "type": "String" },
                    { "name": geometry_property, "type": geometry_type }
                ],
                "allowed_children": allowed_children
            },
            "commit": { "message": format!("create {name}"), "actor": "spatial-query-test" }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("create nodetype {name}: {e}"));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

/// Create a node through the REST node API, which is the path that resolves a
/// parent path into a hierarchy.
async fn create_node(
    fixture: &Fixture,
    workspace: &str,
    parent_path: &str,
    node_type: &str,
    name: &str,
    properties: Value,
) {
    let parent = parent_path.trim_end_matches('/');
    let suffix = if parent.is_empty() {
        "/".to_string()
    } else {
        parent.to_string()
    };
    http_post(
        &fixture.server.base_url,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{workspace}{suffix}"),
        &fixture.token,
        json!({
            "name": name,
            "node_type": node_type,
            "properties": properties,
            "commit": { "message": format!("create {name}"), "actor": "spatial-query-test" }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("create node {parent_path}/{name}: {e}"));
}

fn point(lon: f64, lat: f64) -> Value {
    json!({ "type": "Point", "coordinates": [lon, lat] })
}

/// The `places` fixture: a two-level mall plus two out-of-town nodes.
///
/// | node                | distance from centre | node_type | floor |
/// |---------------------|----------------------|-----------|-------|
/// | `/mall/l1/coffee`   | ~37 m                | q:Shop    | L1    |
/// | `/mall/l1/kiosk`    | ~56 m                | q:Kiosk   | L1    |
/// | `/mall/l2/books`    | ~244 m               | q:Shop    | L2    |
/// | `/depot`            | ~25 km               | q:Shop    | G     |
/// | `/remote`           | ~222 km              | q:Shop    | G     |
async fn seed_places(fixture: &Fixture) {
    create_node(
        fixture,
        PLACES,
        "/",
        "q:Shop",
        "mall",
        json!({ "title": "Mall" }),
    )
    .await;
    create_node(
        fixture,
        PLACES,
        "/mall",
        "q:Shop",
        "l1",
        json!({ "title": "Level 1" }),
    )
    .await;
    create_node(
        fixture,
        PLACES,
        "/mall",
        "q:Shop",
        "l2",
        json!({ "title": "Level 2" }),
    )
    .await;

    create_node(
        fixture,
        PLACES,
        "/mall/l1",
        "q:Shop",
        "coffee",
        json!({ "title": "Coffee", "floor": "L1", "location": point(CENTER_LON, CENTER_LAT + D_37M) }),
    )
    .await;
    create_node(
        fixture,
        PLACES,
        "/mall/l1",
        "q:Kiosk",
        "kiosk",
        json!({ "title": "Kiosk", "floor": "L1", "location": point(CENTER_LON, CENTER_LAT + D_56M) }),
    )
    .await;
    create_node(
        fixture,
        PLACES,
        "/mall/l2",
        "q:Shop",
        "books",
        json!({ "title": "Books", "floor": "L2", "location": point(CENTER_LON, CENTER_LAT + D_244M) }),
    )
    .await;
    create_node(
        fixture,
        PLACES,
        "/",
        "q:Shop",
        "depot",
        json!({ "title": "Depot", "floor": "G", "location": point(CENTER_LON, CENTER_LAT + D_25KM) }),
    )
    .await;
    create_node(
        fixture,
        PLACES,
        "/",
        "q:Shop",
        "remote",
        json!({ "title": "Remote", "floor": "G", "location": point(CENTER_LON, CENTER_LAT + D_222KM) }),
    )
    .await;

    // Give the write-side indexing a moment to settle before querying.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

/// The SQL spelling of the stored geometry.
///
/// `CAST(... AS GEOMETRY)` is spelled out rather than hidden in a helper constant
/// because the *unwrapped* spelling `properties->>'location'` is asserted
/// separately — the planner's `extract_geometry_source` accepts both, and whether
/// the analyzer does is exactly the thing under test.
fn geom() -> String {
    "CAST(properties->>'location' AS GEOMETRY)".to_string()
}

fn center_point() -> String {
    format!("ST_POINT({CENTER_LON}, {CENTER_LAT})")
}

fn dwithin(radius: &str) -> String {
    format!(
        "SELECT name FROM '{PLACES}' WHERE ST_DWITHIN({}, {}, {radius})",
        geom(),
        center_point()
    )
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    out.sort();
    out
}

// --- 1. index-eligible predicate shapes --------------------------------------

/// Every widened predicate shape must return the truth **and** take the intended
/// access path.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn index_eligible_predicate_shapes_return_the_truth_and_use_the_index() {
    let fixture = setup(8341).await;
    seed_places(&fixture).await;

    // Baseline: the one shape that always worked. If this is not index-backed,
    // nothing below is meaningful, so assert it first and loudly.
    let baseline = dwithin("100.0");
    let plan = fixture.explain(&baseline).await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "the canonical ST_DWITHIN spelling must be index-backed on a freshly \
         written workspace — the state record is created by the first geometry \
         write, so `NotBuilt` here means the write path did not register it.\n{plan}"
    );
    assert!(
        !plan.contains("Filter:") && !plan.contains("[with filter]"),
        "an EXACT, coverable ST_DWITHIN is the one case where the predicate MAY \
         leave the residual filter, and it should — otherwise every row the index \
         returns is re-checked for nothing\n{plan}"
    );
    assert_eq!(
        fixture.names(&baseline).await.expect("baseline"),
        sorted(&["coffee", "kiosk"])
    );
    println!("[PASS] baseline ST_DWITHIN: index-backed, predicate stripped, correct rows");

    // The radius window. It used to be silently ~4.8 m - 39 km, and outside it a
    // query returned zero rows; the cover-guaranteed cell plan removed the cliff.
    let scales: &[(&str, &[&str])] = &[
        ("0.5", &[]),
        ("10.0", &[]),
        ("45.0", &["coffee"]),
        ("100.0", &["coffee", "kiosk"]),
        ("500.0", &["coffee", "kiosk", "books"]),
        ("50000.0", &["coffee", "kiosk", "books", "depot"]),
        ("500000.0", &["coffee", "kiosk", "books", "depot", "remote"]),
    ];
    for (radius, expected) in scales {
        let query = dwithin(radius);
        assert_eq!(
            fixture.names(&query).await.expect("radius scale"),
            sorted(expected),
            "wrong rows at radius {radius} m"
        );
    }
    println!("[PASS] radii 0.5 m - 500 km all return the truth");

    // Each entry: (label, WHERE clause, expected names, index-backed, predicate
    // dropped from the residual filter).
    //
    // The last column is THE INVARIANT of the whole subsystem in one boolean: a
    // predicate may leave the residual filter only when the access path is a
    // proven-complete, EXACT answer for it. Every widening below that is merely a
    // superset — a non-point centre reduced to its envelope centre, or a strict
    // `<` whose boundary ring the scan's `<=` includes — must keep it. Asserting
    // only "the index was used" would pass a planner that dropped them all, which
    // is precisely the silent-empty bug in a faster costume.
    let shapes: Vec<(&str, String, Vec<&str>, bool, bool)> = vec![
        (
            "reversed argument order",
            format!("ST_DWITHIN({}, {}, 100.0)", center_point(), geom()),
            vec!["coffee", "kiosk"],
            true,
            true,
        ),
        (
            "integer radius literal",
            format!("ST_DWITHIN({}, {}, 100)", geom(), center_point()),
            vec!["coffee", "kiosk"],
            true,
            true,
        ),
        (
            "ST_MAKEPOINT centre",
            format!(
                "ST_DWITHIN({}, ST_MAKEPOINT({CENTER_LON}, {CENTER_LAT}), 100)",
                geom()
            ),
            vec!["coffee", "kiosk"],
            true,
            true,
        ),
        (
            "ST_GEOMFROMGEOJSON centre",
            format!(
                "ST_DWITHIN({}, ST_GEOMFROMGEOJSON('{{\"type\":\"Point\",\"coordinates\":[{CENTER_LON},{CENTER_LAT}]}}'), 100)",
                geom()
            ),
            vec!["coffee", "kiosk"],
            true,
            true,
        ),
        (
            "ST_SETSRID centre",
            format!(
                "ST_DWITHIN({}, ST_SETSRID({}, 4326), 100)",
                geom(),
                center_point()
            ),
            vec!["coffee", "kiosk"],
            true,
            true,
        ),
        (
            "ST_DISTANCE <= r",
            format!("ST_DISTANCE({}, {}) <= 100", geom(), center_point()),
            vec!["coffee", "kiosk"],
            true,
            true,
        ),
        (
            "ST_DISTANCE < r",
            format!("ST_DISTANCE({}, {}) < 100", geom(), center_point()),
            vec!["coffee", "kiosk"],
            true,
            false,
        ),
        (
            "r > ST_DISTANCE (reversed comparison)",
            format!("100 > ST_DISTANCE({}, {})", geom(), center_point()),
            vec!["coffee", "kiosk"],
            true,
            false,
        ),
        (
            // A non-point centre is reduced to its envelope centre with the radius
            // inflated by the circumradius. That is a strict widening, so it may
            // supply candidates but must NOT strip the predicate — the rows have
            // to come out exactly right anyway.
            "polygon centre (inexact widening)",
            format!(
                "ST_DWITHIN({}, ST_MAKEENVELOPE({}, {}, {}, {}), 100)",
                geom(),
                CENTER_LON - 0.0001,
                CENTER_LAT - 0.0001,
                CENTER_LON + 0.0001,
                CENTER_LAT + 0.0001
            ),
            vec!["coffee", "kiosk"],
            true,
            false,
        ),
        (
            // An anti-range. There is no index path for it, and the important
            // property is that it is not silently dropped: dropping it would
            // return every row instead of the complement.
            "ST_DISTANCE > r (anti-range, no index path)",
            format!("ST_DISTANCE({}, {}) > 100", geom(), center_point()),
            vec!["books", "depot", "remote"],
            false,
            false,
        ),
    ];

    for (label, predicate, expected, index_backed, stripped) in shapes {
        let query = format!("SELECT name FROM '{PLACES}' WHERE {predicate}");
        let rows = fixture
            .names(&query)
            .await
            .unwrap_or_else(|e| panic!("[{label}] query failed: {e}\n  {query}"));
        assert_eq!(rows, sorted(&expected), "[{label}] wrong rows");

        let plan = fixture.explain(&query).await;
        if index_backed {
            assert!(
                plan.contains("SpatialDistanceScan"),
                "[{label}] must be index-backed\n{plan}"
            );
        } else {
            assert!(
                !plan.contains("SpatialDistanceScan"),
                "[{label}] must NOT claim a spatial access path\n{plan}"
            );
        }
        // A retained predicate shows up one of two ways, and BOTH have to be
        // checked or the assertion is vacuous for half the plans: an index scan
        // gets a separate `Filter:` operator above it, while a `TableScan` folds
        // its filter into the scan node and reports `[with filter]`.
        //
        // The absence of both is the planner asserting that the access path alone
        // is the complete, exact answer for the predicate.
        let retained = plan.contains("Filter:") || plan.contains("[with filter]");
        assert_eq!(
            !retained, stripped,
            "[{label}] expected predicate_stripped={stripped}\n{plan}"
        );
        println!("[PASS] {label} (index-backed: {index_backed}, predicate stripped: {stripped})");
    }

    // The unwrapped spelling. `extract_geometry_source` accepts a bare
    // `properties->>'location'`, so whether this works is a question about the
    // analyzer's coercion ladder, not the planner — and it is the spelling a user
    // writes first.
    let unwrapped = format!(
        "SELECT name FROM '{PLACES}' WHERE ST_DWITHIN(properties->>'location', {}, 100)",
        center_point()
    );
    let rows = fixture
        .names(&unwrapped)
        .await
        .unwrap_or_else(|e| panic!("unwrapped properties->>'location' spelling failed: {e}"));
    assert_eq!(rows, sorted(&["coffee", "kiosk"]));
    let plan = fixture.explain(&unwrapped).await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "the unwrapped spelling must reach the index too\n{plan}"
    );
    println!("[PASS] unwrapped properties->>'location' spelling");

    // A bound parameter for the radius. Parameters are substituted before
    // canonicalisation, so this is the same predicate as a literal.
    let parameterised = format!(
        "SELECT name FROM '{PLACES}' WHERE ST_DWITHIN({}, {}, $1)",
        geom(),
        center_point()
    );
    let result = fixture
        .sql_params(&parameterised, vec![json!(100.0)])
        .await
        .unwrap_or_else(|e| panic!("parameterised radius failed: {e}"));
    let mut rows: Vec<String> = result["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| r["name"].as_str().unwrap_or_default().to_string())
        .collect();
    rows.sort();
    assert_eq!(rows, sorted(&["coffee", "kiosk"]), "parameterised radius");
    println!("[PASS] bound-parameter radius");
}

// --- 2. k-NN and distance ordering -------------------------------------------

/// `ORDER BY ST_DISTANCE(...) ASC LIMIT k` must be ordered nearest-first, and
/// must use `SpatialKnnScan` — the plan variant that was unreachable dead code.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn order_by_st_distance_limit_k_is_ordered_and_index_backed() {
    let fixture = setup(8342).await;
    seed_places(&fixture).await;

    let knn = format!(
        "SELECT name FROM '{PLACES}' ORDER BY ST_DISTANCE({}, {}) LIMIT 3",
        geom(),
        center_point()
    );

    let plan = fixture.explain(&knn).await;
    assert!(
        plan.contains("SpatialKnnScan"),
        "ORDER BY ST_DISTANCE ... LIMIT k must plan as a SpatialKnnScan\n{plan}"
    );

    let ordered = fixture.names_in_order(&knn).await.expect("knn query");
    assert_eq!(
        ordered,
        vec![
            "coffee".to_string(),
            "kiosk".to_string(),
            "books".to_string()
        ],
        "k-NN must be nearest-first; the old find_nearest computed its cells and \
         never used them, and its stopping rule answered \"found enough\" rather \
         than \"found the nearest\""
    );
    println!("[PASS] k=3 nearest-first: {ordered:?}");

    // k beyond the number of indexed geometries must return all five, still
    // ordered — a stopping rule that terminates early would truncate here.
    let all = format!(
        "SELECT name FROM '{PLACES}' ORDER BY ST_DISTANCE({}, {}) LIMIT 10",
        geom(),
        center_point()
    );
    assert_eq!(
        fixture.names_in_order(&all).await.expect("knn k=10"),
        vec![
            "coffee".to_string(),
            "kiosk".to_string(),
            "books".to_string(),
            "depot".to_string(),
            "remote".to_string()
        ],
        "k larger than the population must return every geometry, in order"
    );
    println!("[PASS] k=10 returns all five in order");

    // A bounded radius plus an ascending distance order: the scan already emits
    // ascending distance, so the Sort is elided — but only when the ordering is
    // genuinely the one the scan produces.
    let bounded = format!(
        "SELECT name FROM '{PLACES}' WHERE ST_DWITHIN({}, {}, 500) \
         ORDER BY ST_DISTANCE({}, {})",
        geom(),
        center_point(),
        geom(),
        center_point()
    );
    assert_eq!(
        fixture.names_in_order(&bounded).await.expect("bounded knn"),
        vec![
            "coffee".to_string(),
            "kiosk".to_string(),
            "books".to_string()
        ],
        "ST_DWITHIN + ORDER BY ST_DISTANCE must be bounded AND ordered"
    );
    let plan = fixture.explain(&bounded).await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "a bounded ordered query stays a distance scan\n{plan}"
    );
    println!("[PASS] ST_DWITHIN + ORDER BY ST_DISTANCE: bounded and ordered");
}

/// The *unbounded* distance sort: `DESC`, and `ASC` with no LIMIT. Neither has a
/// bounded access path — the k farthest needs every row, and so does an unbounded
/// sort — so both must fall through to `TopN`/`Sort` and still come out ordered.
///
/// Kept as its own test rather than folded into the k-NN one above, because it
/// exercises a DIFFERENT code path (the generic sort over a computed expression,
/// which is not spatial code at all) and a defect there must not be able to mask
/// the k-NN proof.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn an_unbounded_distance_sort_falls_back_and_is_still_ordered() {
    let fixture = setup(8346).await;
    seed_places(&fixture).await;

    let desc = format!(
        "SELECT name FROM '{PLACES}' ORDER BY ST_DISTANCE({}, {}) DESC LIMIT 2",
        geom(),
        center_point()
    );
    let plan = fixture.explain(&desc).await;
    assert!(
        !plan.contains("SpatialKnnScan"),
        "a DESC distance sort must not claim the index's nearest-first order\n{plan}"
    );
    assert_eq!(
        fixture.names_in_order(&desc).await.expect("desc query"),
        vec!["remote".to_string(), "depot".to_string()],
        "farthest-first ordering must still be correct on the fallback path"
    );
    println!("[PASS] DESC distance sort falls back and is still ordered");

    // No LIMIT: every geometry, nearest-first, through the ordinary Sort path.
    let unbounded = format!(
        "SELECT name FROM '{PLACES}' WHERE ST_DWITHIN({}, {}, 500000) \
         ORDER BY ST_DISTANCE({}, {})",
        geom(),
        center_point(),
        geom(),
        center_point()
    );
    assert_eq!(
        fixture
            .names_in_order(&unbounded)
            .await
            .expect("unbounded sort"),
        vec![
            "coffee".to_string(),
            "kiosk".to_string(),
            "books".to_string(),
            "depot".to_string(),
            "remote".to_string()
        ],
        "an unbounded ascending distance sort must return everything, in order"
    );
    println!("[PASS] unbounded ascending distance sort is ordered");
}

// --- 3. composition ----------------------------------------------------------

/// A spatial predicate ANDed with anything else must INTERSECT with it. The
/// planner chooses one access path; every other predicate has to survive as a
/// residual filter or the query silently over- or under-returns.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn spatial_composes_with_node_type_property_and_hierarchy() {
    let fixture = setup(8343).await;
    seed_places(&fixture).await;

    let cases: Vec<(&str, String, Vec<&str>)> = vec![
        (
            "node_type narrows the spatial result",
            format!(
                "ST_DWITHIN({}, {}, 100) AND node_type = 'q:Shop'",
                geom(),
                center_point()
            ),
            vec!["coffee"],
        ),
        (
            "node_type selects the other type",
            format!(
                "ST_DWITHIN({}, {}, 100) AND node_type = 'q:Kiosk'",
                geom(),
                center_point()
            ),
            vec!["kiosk"],
        ),
        (
            "properties->>'floor'::String narrows to one level",
            format!(
                "ST_DWITHIN({}, {}, 500) AND properties->>'floor'::String = 'L1'",
                geom(),
                center_point()
            ),
            vec!["coffee", "kiosk"],
        ),
        (
            "properties->>'floor'::String selects the other level",
            format!(
                "ST_DWITHIN({}, {}, 500) AND properties->>'floor'::String = 'L2'",
                geom(),
                center_point()
            ),
            vec!["books"],
        ),
        (
            "an in-range floor with nothing on it yields nothing",
            format!(
                "ST_DWITHIN({}, {}, 100) AND properties->>'floor'::String = 'L2'",
                geom(),
                center_point()
            ),
            vec![],
        ),
        (
            "CHILD_OF restricts to direct children",
            format!(
                "ST_DWITHIN({}, {}, 500) AND CHILD_OF('/mall/l1')",
                geom(),
                center_point()
            ),
            vec!["coffee", "kiosk"],
        ),
        (
            "DESCENDANT_OF restricts to a subtree",
            format!(
                "ST_DWITHIN({}, {}, 500) AND DESCENDANT_OF('/mall/l2')",
                geom(),
                center_point()
            ),
            vec!["books"],
        ),
        (
            // The load-bearing case: the spatial side matches five nodes and the
            // hierarchy side matches three. Dropping either predicate is
            // observable here and nowhere else.
            "a wide radius intersected with a subtree",
            format!(
                "ST_DWITHIN({}, {}, 500000) AND DESCENDANT_OF('/mall')",
                geom(),
                center_point()
            ),
            vec!["coffee", "kiosk", "books"],
        ),
        (
            "three-way composition",
            format!(
                "ST_DWITHIN({}, {}, 500000) AND DESCENDANT_OF('/mall') \
                 AND node_type = 'q:Shop' AND properties->>'floor'::String = 'L1'",
                geom(),
                center_point()
            ),
            vec!["coffee"],
        ),
    ];

    for (label, predicate, expected) in cases {
        let query = format!("SELECT name FROM '{PLACES}' WHERE {predicate}");
        let rows = fixture
            .names(&query)
            .await
            .unwrap_or_else(|e| panic!("[{label}] failed: {e}\n  {query}"));
        assert_eq!(rows, sorted(&expected), "[{label}] wrong rows\n  {query}");
        println!("[PASS] {label} -> {rows:?}");
    }

    // A control, so a bug that drops the spatial predicate cannot hide behind the
    // hierarchy one: the subtree alone matches the three leaves plus the two
    // structural levels, which the spatial predicate is what removes.
    let subtree_only = format!("SELECT name FROM '{PLACES}' WHERE DESCENDANT_OF('/mall')");
    assert_eq!(
        fixture.names(&subtree_only).await.expect("subtree control"),
        sorted(&["l1", "l2", "coffee", "kiosk", "books"]),
        "control: the subtree alone includes the geometry-less level nodes"
    );
    println!("[PASS] control: DESCENDANT_OF('/mall') alone returns 5 rows");
}

// --- 4. the silent-empty trap ------------------------------------------------

/// A spatial predicate over data the index does not hold must fall back to a
/// row-level filter and return the truth — never zero rows.
///
/// The fixture stores the GeoJSON as a **string** property, so it is a
/// `PropertyValue::String` and nothing spatially indexes it: no index state
/// record for `raw`.`location` exists, the catalog answers `NotBuilt`, and the
/// planner must keep `ST_DWITHIN` as a residual filter. With the old hardcoded
/// `has_spatial_index() == true` this returned zero rows with no error.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn an_unbuilt_spatial_index_falls_back_instead_of_returning_nothing() {
    let fixture = setup(8344).await;

    for (name, lat_offset) in [("here", D_37M), ("there", D_25KM)] {
        let geojson = serde_json::to_string(&point(CENTER_LON, CENTER_LAT + lat_offset)).unwrap();
        create_node(
            &fixture,
            RAW,
            "/",
            "q:Raw",
            name,
            json!({ "title": name, "location": geojson }),
        )
        .await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let query = format!(
        "SELECT name FROM '{RAW}' WHERE ST_DWITHIN(\
         CAST(properties->>'location' AS GEOMETRY), {}, 100)",
        center_point()
    );

    let rows = fixture
        .names(&query)
        .await
        .expect("the fallback path must not error");
    assert_eq!(
        rows,
        vec!["here".to_string()],
        "an unindexed geometry must still be matched by the residual filter — an \
         empty result here IS the silent-empty regression"
    );
    println!("[PASS] unbuilt index still returns the matching row");

    // And the degradation must be VISIBLE, not merely correct.
    let plan = fixture.explain(&query).await;
    assert!(
        plan.contains("TableScan"),
        "an unbuilt index must degrade to a TableScan\n{plan}"
    );
    assert!(
        plan.contains("spatial index NOT USED"),
        "EXPLAIN must name the spatial fallback rather than a generic reason\n{plan}"
    );
    assert!(
        plan.contains("NOT BUILT") && plan.contains("REBUILD SPATIAL INDEX"),
        "EXPLAIN must say what to do about it\n{plan}"
    );
    assert!(
        plan.contains("with filter"),
        "the spatial predicate must survive as a residual filter\n{plan}"
    );
    println!("[PASS] EXPLAIN names the fallback and the remedy");

    // The complement, so "returns the truth" is not satisfied by returning
    // everything: a 100 m radius must exclude the 25 km node.
    let wide = format!(
        "SELECT name FROM '{RAW}' WHERE ST_DWITHIN(\
         CAST(properties->>'location' AS GEOMETRY), {}, 50000)",
        center_point()
    );
    assert_eq!(
        fixture.names(&wide).await.expect("wide fallback query"),
        sorted(&["here", "there"]),
        "the fallback must not under-return either"
    );
    println!("[PASS] fallback is complete at a wider radius too");
}

// --- 5. ST_* over stored geometry, including Multi* --------------------------

/// The ST_\* engine against real stored node data — every geometry type,
/// including `Multi*` and `GeometryCollection`, which no predicate or measurement
/// function accepted as input before the shared conversion layer landed.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn st_functions_round_trip_over_stored_geometry() {
    let fixture = setup(8345).await;

    // Two ~753 m x ~1111 m boxes near (8.60, 47.40); each is ~836_000 m2.
    let multipolygon = json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[8.60, 47.40], [8.61, 47.40], [8.61, 47.41], [8.60, 47.41], [8.60, 47.40]]],
            [[[8.62, 47.40], [8.63, 47.40], [8.63, 47.41], [8.62, 47.41], [8.62, 47.40]]]
        ]
    });
    let shapes: Vec<(&str, Value)> = vec![
        ("pt", point(8.60, 47.40)),
        (
            "pt3d",
            json!({ "type": "Point", "coordinates": [8.64, 47.40, 408.0] }),
        ),
        (
            "line",
            json!({ "type": "LineString", "coordinates": [[8.60, 47.40], [8.60, 47.41]] }),
        ),
        (
            "poly",
            json!({
                "type": "Polygon",
                "coordinates": [[[8.60, 47.40], [8.61, 47.40], [8.61, 47.41], [8.60, 47.41], [8.60, 47.40]]]
            }),
        ),
        (
            "mpt",
            json!({ "type": "MultiPoint", "coordinates": [[8.60, 47.40], [8.61, 47.40], [8.62, 47.40]] }),
        ),
        (
            "mline",
            json!({
                "type": "MultiLineString",
                "coordinates": [[[8.60, 47.40], [8.60, 47.41]], [[8.62, 47.40], [8.62, 47.41]]]
            }),
        ),
        ("mpoly", multipolygon.clone()),
        (
            "gcoll",
            json!({
                "type": "GeometryCollection",
                "geometries": [
                    { "type": "Point", "coordinates": [8.60, 47.40] },
                    { "type": "LineString", "coordinates": [[8.60, 47.40], [8.61, 47.41]] }
                ]
            }),
        ),
        (
            // Topologically invalid (self-intersecting) but syntactically valid
            // GeoJSON. Placed far away so it cannot disturb the radius counts.
            //
            // Deliberately ASYMMETRIC: the textbook bowtie
            // `[[0,0],[1,1],[1,0],[0,1],[0,0]]` has a signed area of exactly zero,
            // which makes its area-weighted centroid a division by zero — a
            // degenerate write fixture, not an invalidity fixture. This one crosses
            // at (0.75, 0.75) and still has an area of 3.
            "bowtie",
            json!({
                "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [3.0, 3.0], [3.0, 0.0], [0.0, 1.0], [0.0, 0.0]]]
            }),
        ),
    ];

    for (name, geometry) in &shapes {
        create_node(
            &fixture,
            SHAPES,
            "/",
            "q:Shape",
            name,
            json!({ "title": name, "geom": geometry }),
        )
        .await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let g = "CAST(properties->>'geom' AS GEOMETRY)";

    // ST_GEOMETRYTYPE over every stored type. A missing row here means the type
    // never round-tripped through storage; an error means a conversion gap.
    let rows = fixture
        .sql(&format!(
            "SELECT name, ST_GEOMETRYTYPE({g}) AS gtype FROM '{SHAPES}' ORDER BY name"
        ))
        .await
        .expect("ST_GEOMETRYTYPE over stored data");
    let mut seen: Vec<(String, String)> = rows["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap_or_default().to_string(),
                r["gtype"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    seen.sort();
    let mut expected: Vec<(String, String)> = vec![
        ("bowtie", "ST_Polygon"),
        ("gcoll", "ST_GeometryCollection"),
        ("line", "ST_LineString"),
        ("mline", "ST_MultiLineString"),
        ("mpoly", "ST_MultiPolygon"),
        ("mpt", "ST_MultiPoint"),
        ("poly", "ST_Polygon"),
        ("pt", "ST_Point"),
        ("pt3d", "ST_Point"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    expected.sort();
    assert_eq!(seen, expected, "every stored geometry type must round-trip");
    println!("[PASS] all 7 GeoJSON types round-trip through storage");

    // Measurement over a stored Multi*. `ST_AREA` on a geographic CRS is square
    // metres, not square degrees — the deliberate divergence from PostGIS's
    // `geometry` type.
    let area = fixture
        .scalar(
            &format!("SELECT ST_AREA({g}) AS a FROM '{SHAPES}' WHERE name = 'mpoly'"),
            "a",
        )
        .await
        .as_f64()
        .expect("ST_AREA over a stored MultiPolygon");
    assert!(
        (1.5e6..1.9e6).contains(&area),
        "two ~836_000 m2 boxes should total ~1.67e6 m2, got {area}"
    );
    println!("[PASS] ST_AREA(stored MultiPolygon) = {area:.0} m2");

    // Both boxes, so a single-polygon reading of a MultiPolygon is caught.
    let single = fixture
        .scalar(
            &format!("SELECT ST_AREA({g}) AS a FROM '{SHAPES}' WHERE name = 'poly'"),
            "a",
        )
        .await
        .as_f64()
        .expect("ST_AREA over a stored Polygon");
    assert!(
        (area / single - 2.0).abs() < 0.05,
        "the MultiPolygon must measure as BOTH rings: {area} vs {single}"
    );
    println!("[PASS] MultiPolygon area is the sum of its parts");

    // ST_LENGTH in metres over a stored MultiLineString: two ~1111 m segments.
    let length = fixture
        .scalar(
            &format!("SELECT ST_LENGTH({g}) AS l FROM '{SHAPES}' WHERE name = 'mline'"),
            "l",
        )
        .await
        .as_f64()
        .expect("ST_LENGTH over a stored MultiLineString");
    assert!(
        (2100.0..2350.0).contains(&length),
        "two 0.01 deg meridian segments are ~2222 m, got {length}"
    );
    println!("[PASS] ST_LENGTH(stored MultiLineString) = {length:.0} m");

    let count = fixture
        .scalar(
            &format!("SELECT ST_NUMGEOMETRIES({g}) AS n FROM '{SHAPES}' WHERE name = 'mpt'"),
            "n",
        )
        .await
        .as_i64()
        .expect("ST_NUMGEOMETRIES over a stored MultiPoint");
    assert_eq!(count, 3);
    println!("[PASS] ST_NUMGEOMETRIES(stored MultiPoint) = 3");

    // Validation over stored data. The old array-shape check passed a bowtie.
    let valid = fixture
        .sql(&format!(
            "SELECT name, ST_ISVALID({g}) AS ok FROM '{SHAPES}' \
             WHERE name = 'bowtie' OR name = 'poly' ORDER BY name"
        ))
        .await
        .expect("ST_ISVALID over stored data");
    let validity: Vec<(String, bool)> = valid["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap_or_default().to_string(),
                r["ok"].as_bool().unwrap_or(true),
            )
        })
        .collect();
    assert_eq!(
        validity,
        vec![("bowtie".to_string(), false), ("poly".to_string(), true)],
        "a self-intersecting stored polygon must be invalid"
    );
    println!("[PASS] ST_ISVALID: stored bowtie invalid, stored box valid");

    // A topological predicate against a stored Multi*, which used to be an
    // "unsupported geometry type" error rather than an answer.
    let inside = fixture
        .scalar(
            &format!(
                "SELECT ST_INTERSECTS({g}, ST_POINT(8.605, 47.405)) AS hit \
                 FROM '{SHAPES}' WHERE name = 'mpoly'"
            ),
            "hit",
        )
        .await;
    assert_eq!(inside, json!(true), "point inside the first box");
    let outside = fixture
        .scalar(
            &format!(
                "SELECT ST_INTERSECTS({g}, ST_POINT(8.615, 47.405)) AS hit \
                 FROM '{SHAPES}' WHERE name = 'mpoly'"
            ),
            "hit",
        )
        .await;
    assert_eq!(
        outside,
        json!(false),
        "the gap between the two boxes is NOT covered — a bounding-box or \
         centroid reading of a MultiPolygon would say true here"
    );
    println!("[PASS] ST_INTERSECTS respects the gap between MultiPolygon parts");

    // A set operation over a stored geometry, then a measurement of the result:
    // `ST_AREA(ST_UNION(a, b))` is the named failure of the old engine, because
    // the union of two DISJOINT polygons is a MultiPolygon and no measurement
    // function accepted one.
    let second_box = serde_json::to_string(&json!({
        "type": "Polygon",
        "coordinates": [[[8.62, 47.40], [8.63, 47.40], [8.63, 47.41], [8.62, 47.41], [8.62, 47.40]]]
    }))
    .unwrap();
    let union_area = fixture
        .scalar(
            &format!(
                "SELECT ST_AREA(ST_UNION({g}, ST_GEOMFROMGEOJSON('{second_box}'))) AS a \
                 FROM '{SHAPES}' WHERE name = 'poly'"
            ),
            "a",
        )
        .await
        .as_f64()
        .expect("ST_AREA(ST_UNION(stored polygon, disjoint literal))");
    assert!(
        (1.5e6..1.9e6).contains(&union_area),
        "ST_AREA(ST_UNION(disjoint boxes)) should be ~1.67e6 m2, got {union_area}"
    );
    assert_eq!(
        fixture
            .scalar(
                &format!(
                    "SELECT ST_GEOMETRYTYPE(ST_UNION({g}, \
                     ST_GEOMFROMGEOJSON('{second_box}'))) AS t \
                     FROM '{SHAPES}' WHERE name = 'poly'"
                ),
                "t"
            )
            .await,
        json!("ST_MultiPolygon"),
        "the union of two disjoint boxes IS a MultiPolygon — if it collapsed to a \
         single Polygon the area assertion above would be measuring the wrong thing"
    );
    println!("[PASS] ST_AREA(ST_UNION(stored, disjoint literal)) = {union_area:.0} m2");

    // Metre-accurate buffering of a stored geometry. `geo`'s Buffer is planar and
    // works in the geometry's own units, so a bare `.buffer(100)` on EPSG:4326
    // would mean 100 DEGREES.
    let buffered = fixture
        .scalar(
            &format!("SELECT ST_AREA(ST_BUFFER({g}, 100)) AS a FROM '{SHAPES}' WHERE name = 'pt'"),
            "a",
        )
        .await
        .as_f64()
        .expect("ST_AREA(ST_BUFFER(stored point, 100))");
    let disc = std::f64::consts::PI * 100.0 * 100.0;
    assert!(
        (buffered / disc - 1.0).abs() < 0.05,
        "a 100 m buffer of a point is a ~{disc:.0} m2 disc, got {buffered}"
    );
    println!("[PASS] ST_BUFFER(stored point, 100) area = {buffered:.0} m2");

    // The third ordinate survives storage: `geo` is 2-D, so Z is read off the
    // representation rather than the geometry.
    assert_eq!(
        fixture
            .scalar(
                &format!("SELECT ST_Z({g}) AS z FROM '{SHAPES}' WHERE name = 'pt3d'"),
                "z"
            )
            .await
            .as_f64(),
        Some(408.0),
        "altitude must survive the write/read round trip"
    );
    assert_eq!(
        fixture
            .scalar(
                &format!("SELECT ST_NDIMS({g}) AS n FROM '{SHAPES}' WHERE name = 'pt3d'"),
                "n"
            )
            .await
            .as_i64(),
        Some(3)
    );
    assert_eq!(
        fixture
            .scalar(
                &format!("SELECT ST_NDIMS({g}) AS n FROM '{SHAPES}' WHERE name = 'pt'"),
                "n"
            )
            .await
            .as_i64(),
        Some(2)
    );
    println!("[PASS] ST_Z / ST_NDIMS over stored 3-D data");

    // SRID is real data: reproject a stored geometry and read the label back.
    assert_eq!(
        fixture
            .scalar(
                &format!(
                    "SELECT ST_SRID(ST_TRANSFORM({g}, 3857)) AS srid FROM '{SHAPES}' \
                     WHERE name = 'mpoly'"
                ),
                "srid"
            )
            .await
            .as_i64(),
        Some(3857),
        "ST_SRID used to be a hardcoded 4326"
    );
    println!("[PASS] ST_SRID(ST_TRANSFORM(stored MultiPolygon, 3857)) = 3857");

    // And finally: a Multi* geometry must be findable through the spatial index.
    // Only the centroid is indexed by default, and the centroid of a
    // GeometryCollection or Multi* used to be computed for Point/LineString/
    // Polygon only — so those nodes were silently absent from every radius query.
    let near_mpoly_centroid = format!(
        "SELECT name FROM '{SHAPES}' WHERE ST_DWITHIN(\
         CAST(properties->>'geom' AS GEOMETRY), ST_POINT(8.615, 47.405), 200)"
    );
    let plan = fixture.explain(&near_mpoly_centroid).await;
    assert!(
        plan.contains("SpatialDistanceScan"),
        "the shapes workspace must be index-backed too\n{plan}"
    );
    let hits = fixture
        .names(&near_mpoly_centroid)
        .await
        .expect("radius query over Multi* data");
    assert!(
        hits.contains(&"mpoly".to_string()),
        "a MultiPolygon must be reachable through the spatial index at its \
         centroid (8.615, 47.405); got {hits:?}"
    );
    println!("[PASS] stored MultiPolygon is index-reachable: {hits:?}");
}
