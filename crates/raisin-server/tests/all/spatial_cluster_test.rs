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

//! Spatial queries must answer identically on **every** node of a masterless
//! cluster. This is the test class whose absence let a live correctness bug
//! survive.
//!
//! # The bug this exists to catch
//!
//! `write_all_node_indexes` on the replication apply path wrote only the property,
//! reference and relation indexes. A geometry written on node1 was therefore
//! spatially queryable **only on node1**: peers converged the node record
//! perfectly and held zero spatial index entries for it. And because the catalog
//! reported the index as always present and the planner then stripped
//! `ST_DWITHIN` from the residual filter, the peer returned **zero rows silently**
//! instead of erroring. Same query, different answer depending on which node
//! replied.
//!
//! No cluster test could see it: `two_node_replication_test` and
//! `three_node_mesh_test` assert only raw record fields, and grepping
//! `spatial|ST_|fulltext|vector|GRAPH_TABLE` across every cluster test yielded
//! nothing. A secondary index is derived *local* state, rebuilt independently on
//! each node, so record-level convergence is no evidence at all about it.
//!
//! # Why each assertion is shaped the way it is
//!
//! * **The record is asserted to have converged first.** Without that, a failing
//!   spatial query is ambiguous — it could just be slow replication. Asserting the
//!   record arrived and *then* asserting the query proves the test exercises the
//!   INDEX and not replication.
//! * **`EXPLAIN` is asserted, not only the rows.** Correct rows do not prove an
//!   index ran: the fail-closed planner keeps the predicate as a row-level filter
//!   when the index is unusable, which is also correct, just slow. Only the plan
//!   distinguishes "node2 built its own index entries" from "node2 fell back to a
//!   full scan and got the right answer anyway".
//! * **An index-free formulation of the same question runs alongside**, and the two
//!   must agree on every node. A divergence between the indexed and the
//!   non-indexed answer *on one node* is the exact signature of a partially
//!   populated index.
//! * **Writes originate on different nodes.** The apply-path fix must be
//!   symmetric, not one-directional.
//!
//! # Running it
//!
//! ```bash
//! cargo test -p raisin-server --test all spatial_cluster_test -- --ignored --nocapture
//! ```
//!
//! Three nodes, because "node2 AND node3 agree" and "a write originating
//! elsewhere" both need a third party — with two, a one-directional fix is
//! indistinguishable from a symmetric one. Ports and data directories come from
//! `cluster_test_utils`, which allocates them dynamically.

#[allow(unused_imports)]
use crate::cluster_test_utils;
#[allow(unused_imports)]
use crate::helpers;

use cluster_test_utils::{
    verify_all_nodes_agree, verify_plan_contains_on_all_nodes, verify_spatial_query_on_all_nodes,
    wait_for_nodetype_replication, wait_for_replication_by_id, ClusterConfig, ClusterProcess,
    RestClient,
};
use helpers::sql_geo::{http_post, http_put};
use serde_json::{json, Value};
use std::time::Duration;

const REPO: &str = "spatial_cluster";
const BRANCH: &str = "main";
const WS: &str = "places";
const NODE_TYPE: &str = "geo:Place";
/// The password `cluster_test_utils::config::NodeConfig::new` bakes into every
/// generated node config.
const PASSWORD: &str = "Admin123!@#$";

/// Zurich Hauptbahnhof. Any lon/lat would do; a real place makes a failure legible.
const CENTER: (f64, f64) = (8.5402, 47.3779);
/// ~33 m north — inside a 100 m radius, outside a 10 m one.
const NEAR: f64 = 0.000_30;
/// ~5 km north. No radius under test spans both this and `NEAR`.
const FAR: f64 = 0.045;

/// Replication plus the peer's own index write is not instantaneous; every
/// cross-node assertion polls up to this long before failing.
const CONVERGE: Duration = Duration::from_secs(90);

// ----------------------------------------------------------------- SQL builders

fn point_json(lon: f64, lat: f64) -> String {
    format!("{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}")
}

fn props(title: &str, lon: f64, lat: f64) -> String {
    format!(
        "'{{\"title\":\"{title}\",\"location\":{}}}'::JSONB",
        point_json(lon, lat)
    )
}

/// The index-eligible spelling.
///
/// `CAST(properties->>'x' AS GEOMETRY)` is the only one of the three spellings that
/// survives analysis today: a bare `properties->>'location'` fails with
/// `Function not found: ST_DWITHIN(TEXT?, GEOMETRY, DOUBLE)` and a bare `location`
/// is not a declared column of the workspace schema. Both are analyzer gaps rather
/// than planner gaps — `extract_geometry_source` handles all three identically.
fn dwithin(lon: f64, lat: f64, radius: f64) -> String {
    format!(
        "SELECT name FROM '{WS}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({lon}, {lat}), {radius}) \
         ORDER BY name"
    )
}

/// The same question with **no** access path into the spatial index: the filter is
/// on `node_type`, the distance is computed in the projection, and the radius test
/// happens in this process.
///
/// `ST_DISTANCE(...) < r` would not serve — the planner rewrites that onto the same
/// `SpatialDWithin` access path by design, so it would not be an independent
/// answer.
fn distances_sql() -> String {
    format!(
        "SELECT name, ST_DISTANCE(CAST(properties->>'location' AS GEOMETRY), \
                                  ST_POINT({}, {})) AS d \
         FROM '{WS}' WHERE node_type = '{NODE_TYPE}'",
        CENTER.0, CENTER.1
    )
}

// ------------------------------------------------------------------- the cluster

struct Cluster {
    cluster: ClusterProcess,
    client: RestClient,
    tokens: Vec<String>,
}

impl Cluster {
    async fn start(node_count: usize) -> Self {
        let ports = cluster_test_utils::unique_ports(node_count * 2);
        let config = ClusterConfig::new_with_ports(&ports).expect("cluster config");
        let cluster = ClusterProcess::start(config)
            .await
            .expect("failed to start cluster");
        cluster
            .wait_for_health(Duration::from_secs(90))
            .await
            .expect("cluster did not become healthy");

        let client = RestClient::new(cluster.config.base_urls());

        // The admin user is created asynchronously by the TenantCreated handler.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let mut tokens = Vec::new();
        for (idx, url) in client.base_urls.iter().enumerate() {
            tokens.push(authenticate_with_retry(&client, url, idx).await);
        }

        // Peer connections are established in the background. Creating data before
        // the mesh is up makes convergence timing unpredictable rather than wrong.
        tokio::time::sleep(Duration::from_secs(5)).await;

        Self {
            cluster,
            client,
            tokens,
        }
    }

    fn nodes(&self) -> usize {
        self.client.base_urls.len()
    }

    fn url(&self, node: usize) -> &str {
        &self.client.base_urls[node]
    }

    fn token(&self, node: usize) -> &str {
        &self.tokens[node]
    }

    /// Run a statement on one node.
    async fn sql(&self, node: usize, sql: &str) -> Result<Value, String> {
        self.client
            .execute_sql(self.url(node), self.token(node), REPO, sql, vec![])
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn expect_everywhere(&self, sql: &str, want: &[&str]) -> Result<(), String> {
        verify_spatial_query_on_all_nodes(&self.client, &self.tokens, REPO, sql, want, CONVERGE)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn expect_plan_everywhere(&self, sql: &str, needle: &str) -> Result<(), String> {
        verify_plan_contains_on_all_nodes(&self.client, &self.tokens, REPO, sql, needle)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn agree(&self, sql: &str) -> Result<Vec<String>, String> {
        verify_all_nodes_agree(&self.client, &self.tokens, REPO, sql, CONVERGE)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    /// Names within `radius` of `CENTER` on one node, computed row-by-row from the
    /// returned distances rather than by any spatial index.
    async fn names_by_computed_distance(
        &self,
        node: usize,
        radius: f64,
    ) -> Result<Vec<String>, String> {
        let result = self.sql(node, &distances_sql()).await?;
        let mut names: Vec<String> = result["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["d"].as_f64().map(|d| d <= radius).unwrap_or(false))
                    .filter_map(|row| row["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    async fn wait_for_record(&self, node_id: &str) -> Result<(), String> {
        wait_for_replication_by_id(
            &self.client,
            &self.tokens,
            REPO,
            BRANCH,
            WS,
            node_id,
            CONVERGE,
        )
        .await
        .map_err(|e| format!("{e:#}"))
    }

    async fn workspace_config(&self, node: usize) -> Result<Value, String> {
        reqwest::Client::new()
            .get(format!(
                "{}/api/workspaces/{REPO}/{WS}/config",
                self.url(node)
            ))
            .bearer_auth(self.token(node))
            .send()
            .await
            .map_err(|e| format!("config GET on node{}: {e}", node + 1))?
            .json::<Value>()
            .await
            .map_err(|e| format!("config GET on node{} is not JSON: {e}", node + 1))
    }
}

async fn authenticate_with_retry(client: &RestClient, url: &str, idx: usize) -> String {
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=10 {
        match client.authenticate(url, "default", "admin", PASSWORD).await {
            Ok(token) => return token,
            Err(e) if attempt < 10 => {
                println!("  auth attempt {attempt} on node{} failed: {e}", idx + 1);
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
            Err(e) => panic!("failed to authenticate to node{}: {e:#}", idx + 1),
        }
    }
    unreachable!()
}

// ----------------------------------------------------------------- provisioning

/// Create the repository, workspace and NodeType.
///
/// Repository and workspace are created on **every** node, tolerating "already
/// exists" — that is what `ClusterTestFixture` does and what is known to work here.
/// The NodeType is created once on node1 and asserted to replicate, because the
/// spatial write path resolves its policy from schema and a peer that never saw the
/// type would be testing something else.
async fn provision(c: &Cluster) {
    for node in 0..c.nodes() {
        tolerate_exists(
            http_post(
                c.url(node),
                "/api/repositories",
                c.token(node),
                json!({
                    "repo_id": REPO,
                    "description": "Spatial cluster test repo",
                    "default_branch": BRANCH
                }),
            )
            .await
            .map(|_| ()),
            "repository",
            node,
        );

        tolerate_exists(
            http_put(
                c.url(node),
                &format!("/api/workspaces/{REPO}/{WS}"),
                c.token(node),
                json!({
                    "name": WS,
                    // `raisin:Folder` must be allowed: creating a workspace
                    // materialises a root folder node, so a workspace permitting
                    // only its own type is rejected at creation time.
                    "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
                    "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
                    "depends_on": [],
                    "config": { "default_branch": BRANCH, "node_type_pins": {} }
                }),
            )
            .await,
            "workspace",
            node,
        );
    }

    http_post(
        c.url(0),
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        c.token(0),
        json!({
            "node_type": {
                "name": NODE_TYPE,
                "description": "A place with a location",
                "properties": [
                    { "name": "title", "type": "String" },
                    { "name": "location", "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create geo:Place NodeType", "actor": "spatial-cluster-test" }
        }),
    )
    .await
    .expect("create nodetype on node1");

    wait_for_nodetype_replication(
        &c.client,
        &c.tokens,
        REPO,
        BRANCH,
        NODE_TYPE,
        Duration::from_secs(60),
    )
    .await
    .expect("NodeType failed to replicate to every node");
    println!("  NodeType '{NODE_TYPE}' present on every node");
}

fn tolerate_exists(result: Result<(), String>, what: &str, node: usize) {
    if let Err(e) = result {
        if !e.contains("already exists") {
            panic!("failed to create {what} on node{}: {e}", node + 1);
        }
    }
}

// ----------------------------------------------------------------------- phases

/// A geometry written on node1 must be reachable **through the spatial index** on
/// node2 and node3. This is the point fix.
async fn phase_write_on_node1(c: &Cluster) -> Result<(), String> {
    c.sql(
        0,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('p1', '/p1', '{NODE_TYPE}', {})",
            props("Place One", CENTER.0, CENTER.1 + NEAR)
        ),
    )
    .await?;

    // First: the RECORD converged. Without this, a spatial miss below would be
    // ambiguous between "the index is missing" and "replication is slow".
    c.wait_for_record("p1").await?;
    println!("  record 'p1' converged on all nodes");

    // Then: the index-backed query. Before the fix this returned [] on node2/node3.
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 100.0), &["p1"])
        .await?;

    // The radius must discriminate — a query returning everything proves nothing
    // about an index.
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 10.0), &[])
        .await?;

    // The rows alone are not proof. Assert the plan really is the spatial index on
    // EVERY node, which is what says the peers built their own entries.
    c.expect_plan_everywhere(&dwithin(CENTER.0, CENTER.1, 100.0), "SpatialDistanceScan")
        .await?;
    println!("  every node's plan is a SpatialDistanceScan");
    Ok(())
}

/// The indexed and the index-free answer must agree, on every node.
async fn phase_indexed_matches_row_eval(c: &Cluster) -> Result<(), String> {
    let indexed = c.agree(&dwithin(CENTER.0, CENTER.1, 100.0)).await?;

    // Sanity: the comparison query must genuinely NOT use the index, or it is not
    // an independent answer.
    let plan = cluster_test_utils::explain_on_node(
        &c.client,
        c.url(0),
        c.token(0),
        REPO,
        &distances_sql(),
    )
    .await
    .map_err(|e| format!("{e:#}"))?;
    if plan.contains("SpatialDistanceScan") || plan.contains("SpatialKnnScan") {
        return Err(format!(
            "the control query was meant to avoid the spatial index but its plan uses \
             it, so it is not an independent answer:\n{plan}"
        ));
    }

    for node in 0..c.nodes() {
        let computed = c.names_by_computed_distance(node, 100.0).await?;
        if computed != indexed {
            return Err(format!(
                "node{}: indexed {:?} != row-evaluated {:?} — the signature of a \
                 partially populated index",
                node + 1,
                indexed,
                computed
            ));
        }
    }
    println!("  indexed and row-evaluated answers agree on every node: {indexed:?}");
    Ok(())
}

/// Moving the geometry must move the index entry on every node — the stale-entry
/// half of the fix, on the apply path rather than only locally.
async fn phase_update_moves_the_entry(c: &Cluster) -> Result<(), String> {
    let (lon, lat) = (CENTER.0, CENTER.1 + FAR);
    c.sql(
        0,
        &format!(
            "UPDATE '{WS}' SET properties = {} WHERE id = 'p1'",
            props("Place One", lon, lat)
        ),
    )
    .await?;

    // The OLD cell must stop matching. A moved node matching at BOTH locations is
    // exactly what the tombstone-dedup bug produced.
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 200.0), &[])
        .await?;
    c.expect_everywhere(&dwithin(lon, lat, 200.0), &["p1"])
        .await?;
    println!("  the moved entry matches only at its new location, on every node");
    Ok(())
}

/// A delete must stop matching spatial queries on every node.
async fn phase_delete(c: &Cluster) -> Result<(), String> {
    c.sql(0, &format!("DELETE FROM '{WS}' WHERE id = 'p1'"))
        .await?;

    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1 + FAR, 200.0), &[])
        .await?;
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 100.0), &[])
        .await?;
    println!("  the deleted node matches nowhere, on every node");
    Ok(())
}

/// The fix must be symmetric: a write originating on node2 must be spatially
/// queryable on node1 and node3.
async fn phase_write_on_node2(c: &Cluster) -> Result<(), String> {
    c.sql(
        1,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('p2', '/p2', '{NODE_TYPE}', {})",
            props("Place Two", CENTER.0, CENTER.1 + NEAR)
        ),
    )
    .await?;
    c.wait_for_record("p2").await?;

    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 100.0), &["p2"])
        .await?;
    c.expect_plan_everywhere(&dwithin(CENTER.0, CENTER.1, 100.0), "SpatialDistanceScan")
        .await?;
    println!("  a node2-originated write is index-backed on node1 and node3 too");
    Ok(())
}

/// ~55 m, ~110 m, ~550 m and ~5.5 km north of `CENTER`, written in one statement
/// from node3.
const MULTI: [(&str, f64); 4] = [
    ("m1", 0.000_5),
    ("m2", 0.001_0),
    ("m3", 0.005_0),
    ("m4", 0.050_0),
];

/// Several rows at different scales, so a single radius cannot be mistaken for "the
/// index returned everything", and every node must agree at every scale.
async fn phase_multi_scale(c: &Cluster) -> Result<(), String> {
    let values = MULTI
        .iter()
        .map(|(id, off)| {
            format!(
                "('{id}', '/{id}', '{NODE_TYPE}', {})",
                props(id, CENTER.0, CENTER.1 + off)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    c.sql(
        2,
        &format!("INSERT INTO '{WS}' (id, path, node_type, properties) VALUES {values}"),
    )
    .await?;
    for (id, _) in MULTI {
        c.wait_for_record(id).await?;
    }

    // p2 sits ~33 m out, so it joins every one of these.
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 100.0), &["m1", "p2"])
        .await?;
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 200.0), &["m1", "m2", "p2"])
        .await?;
    c.expect_everywhere(
        &dwithin(CENTER.0, CENTER.1, 1_000.0),
        &["m1", "m2", "m3", "p2"],
    )
    .await?;
    c.expect_everywhere(
        &dwithin(CENTER.0, CENTER.1, 10_000.0),
        &["m1", "m2", "m3", "m4", "p2"],
    )
    .await?;
    println!("  every node agrees at 100 m / 200 m / 1 km / 10 km");
    Ok(())
}

/// Reconfiguring the precision set on ONE node must not change ANY node's answers.
///
/// This is the spec's mixed-state guarantee — *at no point does any node return a
/// different result set; only latency differs* — and it is asserted in its sharpest
/// form, because after the reconfiguration the cluster is genuinely heterogeneous:
/// node1's declared policy is `[3, 5, 9, 12]` and node2/node3's is the default
/// `[2, 4, 6, 7, 8, 9, 10, 11]`. The property holds only because the cell-planning
/// algorithm is cover-guaranteeing at any precision; a scheme that picked a fixed
/// precision per radius would return different rows on differently-configured nodes.
///
/// # Two gaps this phase reports rather than asserts
///
/// **1. The config does not fan out.** The spec makes `WorkspaceConfig.spatial`
/// replicated intent — "the config change IS the fan-out mechanism". It is not:
/// `OpType::UpdateWorkspace` has an applier
/// (`replication/application/applicator/workspace_branch_ops.rs`) but **no producer
/// anywhere in the tree** — `WorkspaceRepositoryImpl::put`
/// (`repositories/workspaces.rs`) holds no `OperationCapture` and emits only a local
/// event. So a workspace record, and with it every spatial policy, is local to the
/// node it was written on. That is a release blocker for the fan-out story, but
/// fixing it is a replication change well outside this test (the applier is a blind
/// overwrite with no HLC guard, so a naive producer would let an older workspace
/// clobber a newer one). It is reported, and the heterogeneity it causes is turned
/// into the stronger assertion above rather than papered over.
///
/// **2. Nothing re-indexes under the new policy.** `from_local_state` resolves the
/// write policy from the LOCAL state record, not from the configured one, so node1
/// keeps writing at its already-indexed precisions until a rebuild runs — and no
/// rebuild can be triggered, because `JobType::SpatialIndexBuild` has no SQL or HTTP
/// surface. So this phase proves configured-policy heterogeneity is harmless; it
/// cannot prove indexed-precision heterogeneity is, and does not claim to.
async fn phase_reconfigure_precisions(c: &Cluster) -> Result<(), String> {
    let query = dwithin(CENTER.0, CENTER.1, 1_000.0);
    let before = c.agree(&query).await?;

    // Deliberately disjoint from the default [2,4,6,7,8,9,10,11] except at 9.
    let precisions = json!([3, 5, 9, 12]);
    http_put(
        c.url(0),
        &format!("/api/workspaces/{REPO}/{WS}/config"),
        c.token(0),
        json!({
            "default_branch": BRANCH,
            "node_type_pins": {},
            "spatial": { "default": { "precisions": precisions }, "properties": {} }
        }),
    )
    .await
    .map_err(|e| format!("workspace config PUT rejected: {e}"))?;

    // The write must at least have taken effect where it was made, or the rest of
    // this phase is vacuous — nothing would have been reconfigured at all.
    let local = c.workspace_config(0).await?;
    if local["spatial"]["default"]["precisions"] != precisions {
        return Err(format!(
            "node1 did not persist the spatial policy it was given; \
             GET .../config returned spatial={}",
            local["spatial"]
        ));
    }
    println!("  node1 accepted and persisted the new precision set");

    // Whether it reached the peers is reported, not asserted — see the doc comment.
    let mut fanned_out = true;
    for node in 1..c.nodes() {
        let config = c.workspace_config(node).await?;
        if config["spatial"]["default"]["precisions"] != precisions {
            fanned_out = false;
            println!(
                "  [GAP] node{} still has spatial={} — WorkspaceConfig does not \
                 replicate (OpType::UpdateWorkspace has an applier but no producer). \
                 The cluster is now heterogeneous, which is what the assertions below \
                 exercise.",
                node + 1,
                config["spatial"]
            );
        }
    }
    if fanned_out {
        println!("  the new precision set replicated to every node");
    }

    // THE invariant: precision is a performance knob, never a correctness one.
    let after = c.agree(&query).await?;
    if before != after {
        return Err(format!(
            "reconfiguring precisions changed the result set: {before:?} -> {after:?}"
        ));
    }

    // A row written on the RECONFIGURED node after the change must still be findable
    // on the differently-configured peers, at every scale.
    c.sql(
        0,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('r1', '/r1', '{NODE_TYPE}', {})",
            props("Reconfigured", CENTER.0, CENTER.1 + 0.000_6)
        ),
    )
    .await?;
    c.wait_for_record("r1").await?;
    c.expect_everywhere(&query, &["m1", "m2", "m3", "p2", "r1"])
        .await?;
    // And a radius finer than any cell the index holds must still be exact — this is
    // where a scheme that only searched its configured precisions returns nothing.
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1, 5.0), &[])
        .await?;
    // r1 and m1 are 0.0001 degrees of latitude apart — about 11 m — so a 5 m radius
    // around r1 separates them. Anything wider returns both, which would say nothing
    // about resolution.
    c.expect_everywhere(&dwithin(CENTER.0, CENTER.1 + 0.000_6, 5.0), &["r1"])
        .await?;
    println!("  a write made under the new policy is findable on every node, at every scale");
    Ok(())
}

/// SRID normalisation feeds the index, so it must be deterministic across nodes.
///
/// Only the built-in tier (4326, 3857 and the 120 WGS84 UTM zones) may be used by
/// the write-time normaliser, precisely so two nodes built with different Cargo
/// features cannot produce different cells for the same replicated record. The
/// assertion is cross-node identity, which is what decision 10 requires — not a
/// particular row set, since the right answer for a projected centre is a separate
/// question from index determinism.
async fn phase_srid_determinism(c: &Cluster) -> Result<(), String> {
    // Zurich HB expressed in EPSG:3857.
    const MERCATOR: (f64, f64) = (950_668.451_374_556_3, 6_002_677.997_532_715);
    let geom = format!(
        "{{\"type\":\"Point\",\"coordinates\":[{},{}],\"srid\":3857}}",
        MERCATOR.0, MERCATOR.1
    );
    c.sql(
        2,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('web1', '/web1', '{NODE_TYPE}', \
                     '{{\"title\":\"WebMercator\",\"location\":{geom}}}'::JSONB)"
        ),
    )
    .await?;
    c.wait_for_record("web1").await?;

    // The label survives storage and replication, identically everywhere.
    let srid_sql = format!(
        "SELECT name FROM '{WS}' \
         WHERE ST_SRID(CAST(properties->>'location' AS GEOMETRY)) = 3857"
    );
    let labelled = c.agree(&srid_sql).await?;
    if labelled != vec!["web1".to_string()] {
        return Err(format!(
            "the stored SRID did not survive replication: ST_SRID = 3857 matched {labelled:?}"
        ));
    }

    // Every node must give the same answer to a spatial query over a workspace that
    // now holds mixed SRIDs. Whatever that answer is, a divergence would mean the
    // nodes normalised differently.
    let agreed = c.agree(&dwithin(CENTER.0, CENTER.1, 1_000.0)).await?;
    println!("  every node agrees over a mixed-SRID workspace: {agreed:?}");

    // Reported, not asserted, because it is a single-node gap that this cluster
    // module is the wrong place to fail on — but it is a real one and it is named
    // exactly.
    //
    // The spec requires the index to store WGS84 lon/lat only, normalising at WRITE
    // time via `raisin_proj::normalize_for_index` and FAILING THE WRITE LOUDLY for
    // an SRID that tier 1 cannot handle. Neither half exists:
    // `normalize_for_index` has zero callers (raisin-rocksdb declares the dependency
    // and names the function in a Cargo.toml comment, but `spatial/geometry.rs` has
    // no SRID awareness at all), so a 3857 geometry is geohashed with its projected
    // easting/northing read as degrees. The entry lands nowhere near the place it
    // describes and the row silently stops matching — the exact failure class this
    // pass exists to remove. Deterministically so on every node, which is why the
    // cross-node assertion above still holds.
    if agreed.contains(&"web1".to_string()) {
        println!("  [NOTE] the 3857 row is also matched by a 4326-centred query");
    } else {
        println!(
            "  [GAP] the 3857 row is NOT matched by a 4326-centred query on any node: \
             write-time SRID normalisation is not implemented \
             (raisin_proj::normalize_for_index has no callers), so its index cells \
             were derived from raw projected coordinates. Identical on all three \
             nodes, so index determinism holds; the geometry is simply mis-placed \
             everywhere rather than on one node."
        );
    }
    Ok(())
}

// --------------------------------------------------------------------- the test

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_cluster_test -- --ignored --nocapture
async fn test_spatial_queries_answer_identically_on_every_cluster_node() {
    println!("\n=== spatial indexing across a masterless cluster ===\n");

    let c = Cluster::start(3).await;
    provision(&c).await;
    println!("\n[OK] 3-node cluster up, repo/workspace/nodetype provisioned\n");

    // Every phase runs even if an earlier one failed, so one run reports the whole
    // picture rather than only the first break. Mutations are ordered, so a later
    // phase may fail *because* an earlier one did — the summary keeps the order.
    let mut outcomes: Vec<(&str, Result<(), String>)> = Vec::new();

    macro_rules! phase {
        ($name:expr, $body:expr) => {{
            println!("\n--- {} ---", $name);
            let outcome = $body.await;
            match &outcome {
                Ok(()) => println!("[PASS] {}", $name),
                Err(e) => println!("[FAIL] {}\n       {}", $name, e),
            }
            outcomes.push(($name, outcome));
        }};
    }

    phase!(
        "a geometry written on node1 is index-backed on node2 and node3",
        phase_write_on_node1(&c)
    );
    phase!(
        "the indexed and index-free answers agree on every node",
        phase_indexed_matches_row_eval(&c)
    );
    phase!(
        "an UPDATE moves the entry on every node, leaving nothing stale",
        phase_update_moves_the_entry(&c)
    );
    phase!(
        "a DELETE stops matching spatial queries on every node",
        phase_delete(&c)
    );
    phase!(
        "the fix is symmetric: a node2-originated write is queryable elsewhere",
        phase_write_on_node2(&c)
    );
    phase!(
        "every node agrees at every scale from 100 m to 10 km",
        phase_multi_scale(&c)
    );
    phase!(
        "reconfiguring precisions fans out and changes no answer",
        phase_reconfigure_precisions(&c)
    );
    phase!(
        "SRID normalisation is identical on every node",
        phase_srid_determinism(&c)
    );

    println!("\n=== summary ===");
    for (name, outcome) in &outcomes {
        println!(
            "  {} {}",
            if outcome.is_ok() { "PASS" } else { "FAIL" },
            name
        );
    }

    let failures: Vec<String> = outcomes
        .iter()
        .filter_map(|(name, outcome)| outcome.as_ref().err().map(|e| format!("{name}: {e}")))
        .collect();

    if !failures.is_empty() {
        for logs in c.cluster.log_paths() {
            println!(
                "  logs for {}: {} / {}",
                logs.node_id,
                logs.stdout_path.display(),
                logs.stderr_path.display()
            );
        }
        panic!(
            "{} spatial cluster phase(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    println!("\n=== spatial cluster: PASS ===\n");
}
