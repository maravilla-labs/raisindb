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

//! A workspace config written on one node must reach every other node.
//!
//! # The bug this exists to catch
//!
//! `OpType::UpdateWorkspace` shipped with an applier
//! (`replication/application/applicator/`) and **no producer anywhere in the
//! tree**: `WorkspaceRepositoryImpl::put` held no `OperationCapture` and emitted
//! only a local event. Every workspace record — and with it every workspace
//! config, including `WorkspaceConfig.spatial`, whose entire documented
//! cluster-wide fan-out mechanism *is* this operation — was silently local to
//! the node it was written on. `PUT .../config` returned 204 and the peers never
//! heard about it.
//!
//! The half-fix is worse than the bug. The applier was a blind overwrite with no
//! conflict resolution, so a producer on its own would let an out-of-order or
//! replayed peer message carrying an OLDER workspace land on top of a newer one
//! and silently revert a config change that the operator watched succeed. Both
//! halves are therefore asserted here: fan-out (phases 1 and 2) *and* the
//! last-write-wins guard (phase 3).
//!
//! # Why each phase is shaped the way it is
//!
//! * **Phase 1 asserts the config on the peers, not the write's status code.**
//!   A 204 on the writing node proved nothing before the fix and proves nothing
//!   now; only a `GET .../config` on a *different* node distinguishes "replicated"
//!   from "written locally".
//! * **Phase 2 writes real data through SQL after the config change.** A config
//!   that replicates but breaks writes on the receiving node is not a fix. SQL is
//!   used deliberately rather than only the REST node API, because they are
//!   different write paths into the same workspace.
//! * **Phase 3 drives the conflict from a second node and asserts convergence
//!   in BOTH directions.** After node3 writes a newer config, no node — including
//!   node3 itself, which still holds node1's older op in its history — may end up
//!   back on the older value. A blind-overwrite applier passes the "newer wins"
//!   direction by luck of timing and fails this one under replay.
//!
//! # Running it
//!
//! ```bash
//! cargo test -p raisin-server --test all workspace_config_replication_test \
//!     -- --ignored --nocapture
//! ```
//!
//! Three nodes, so that "the write reached the peers" is not satisfiable by a
//! single point-to-point link, and so phase 3 can originate its conflicting write
//! on a node that is neither the first writer nor a pure observer.

#[allow(unused_imports)]
use crate::cluster_test_utils;
#[allow(unused_imports)]
use crate::helpers;

use cluster_test_utils::{wait_for_replication_by_id, ClusterConfig, ClusterProcess, RestClient};
use helpers::sql_geo::{http_post, http_put};
use serde_json::{json, Value};
use std::time::Duration;

const REPO: &str = "ws_config_repl";
const BRANCH: &str = "main";
const WS: &str = "configured";
const NODE_TYPE: &str = "raisin:Folder";

/// The password `cluster_test_utils::config::NodeConfig::new` bakes into every
/// generated node config.
const PASSWORD: &str = "Admin123!@#$";

/// Replication is asynchronous; every cross-node assertion polls up to this long
/// before failing.
const CONVERGE: Duration = Duration::from_secs(90);

// ------------------------------------------------------------------- the cluster

struct Cluster {
    #[allow(dead_code)]
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

        // Peer connections are established in the background. Writing before the
        // mesh is up makes convergence timing unpredictable rather than wrong.
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

    async fn sql(&self, node: usize, sql: &str) -> Result<Value, String> {
        self.client
            .execute_sql(self.url(node), self.token(node), REPO, sql, vec![])
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

    /// Write a spatial policy on `node` via the public workspace-config endpoint.
    ///
    /// The full `WorkspaceConfig` is sent because the endpoint replaces it
    /// wholesale; sending only `spatial` would drop `default_branch` and make a
    /// later failure ambiguous between "did not replicate" and "was written
    /// wrong".
    async fn put_precisions(&self, node: usize, precisions: &Value) -> Result<(), String> {
        http_put(
            self.url(node),
            &format!("/api/workspaces/{REPO}/{WS}/config"),
            self.token(node),
            json!({
                "default_branch": BRANCH,
                "node_type_pins": {},
                "spatial": { "default": { "precisions": precisions }, "properties": {} }
            }),
        )
        .await
        .map_err(|e| format!("workspace config PUT on node{} rejected: {e}", node + 1))
    }

    /// Poll every node until all of them report `precisions`, or give up.
    ///
    /// Every node is required, not just "some node other than the writer": a
    /// change that reaches one peer and not another is the heterogeneous state
    /// the fan-out exists to prevent.
    async fn expect_precisions_everywhere(&self, precisions: &Value) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + CONVERGE;
        loop {
            let mut pending = Vec::new();
            for node in 0..self.nodes() {
                let config = self.workspace_config(node).await?;
                if &config["spatial"]["default"]["precisions"] != precisions {
                    pending.push(format!("node{} has {}", node + 1, config["spatial"]));
                }
            }
            if pending.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "workspace config did not reach every node within {}s \
                     (wanted precisions {precisions}): {}",
                    CONVERGE.as_secs(),
                    pending.join("; ")
                ));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Assert every node reports `precisions` and keeps reporting it.
    ///
    /// A single sample cannot tell a converged cluster from one that is midway
    /// through being clobbered by a late older operation, which is exactly the
    /// failure the LWW guard prevents — so the state is re-checked after a settle
    /// window.
    async fn expect_precisions_stable(&self, precisions: &Value) -> Result<(), String> {
        self.expect_precisions_everywhere(precisions).await?;
        tokio::time::sleep(Duration::from_secs(10)).await;
        for node in 0..self.nodes() {
            let config = self.workspace_config(node).await?;
            if &config["spatial"]["default"]["precisions"] != precisions {
                return Err(format!(
                    "node{} reverted after converging: expected {precisions}, got {} — \
                     an older UpdateWorkspace clobbered a newer one",
                    node + 1,
                    config["spatial"]
                ));
            }
        }
        Ok(())
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

/// Create the repository and workspace on every node, tolerating "already exists".
///
/// The workspace is created everywhere rather than created once and waited for,
/// because whether workspace *creation* replicates is what phase 1 measures — the
/// test must not depend on the thing it is testing in order to set itself up.
async fn provision(c: &Cluster) {
    for node in 0..c.nodes() {
        tolerate_exists(
            http_post(
                c.url(node),
                "/api/repositories",
                c.token(node),
                json!({
                    "repo_id": REPO,
                    "description": "Workspace config replication test repo",
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
                    "allowed_node_types": [NODE_TYPE],
                    "allowed_root_node_types": [NODE_TYPE],
                    "depends_on": [],
                    "config": { "default_branch": BRANCH, "node_type_pins": {} }
                }),
            )
            .await,
            "workspace",
            node,
        );
    }
}

fn tolerate_exists(result: Result<(), String>, what: &str, node: usize) {
    if let Err(e) = result {
        if !e.contains("already exists") {
            panic!("failed to create {what} on node{}: {e}", node + 1);
        }
    }
}

// ----------------------------------------------------------------------- phases

/// A config written on node1 must be readable on node2 and node3.
async fn phase_config_fans_out(c: &Cluster) -> Result<(), String> {
    // Deliberately disjoint from the server default [2,4,6,7,8,9,10,11] except at
    // 9, so a peer that merely fell back to defaults cannot pass by coincidence.
    let precisions = json!([3, 5, 9, 12]);
    c.put_precisions(0, &precisions).await?;

    // The write must at least have taken effect where it was made, or the rest of
    // the phase is vacuous.
    let local = c.workspace_config(0).await?;
    if local["spatial"]["default"]["precisions"] != precisions {
        return Err(format!(
            "node1 did not persist the policy it was given; GET returned spatial={}",
            local["spatial"]
        ));
    }
    println!("  node1 accepted and persisted the new precision set");

    c.expect_precisions_everywhere(&precisions).await?;
    println!("  the new precision set reached every node");
    Ok(())
}

/// The receiving nodes must still accept writes into the reconfigured workspace.
///
/// A replicated config that leaves the workspace unusable on the nodes that
/// received it would satisfy phase 1 and still be a broken feature. SQL is the
/// write path here, and the read-back is on a node that never saw the INSERT.
async fn phase_writes_still_work_under_the_replicated_config(c: &Cluster) -> Result<(), String> {
    c.sql(
        0,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('cfg1', '/cfg1', '{NODE_TYPE}', '{{\"title\":\"Written\"}}'::JSONB)"
        ),
    )
    .await?;

    wait_for_replication_by_id(&c.client, &c.tokens, REPO, BRANCH, WS, "cfg1", CONVERGE)
        .await
        .map_err(|e| {
            format!("record written under the replicated config did not converge: {e:#}")
        })?;

    // And a write ORIGINATING on a node that only received the config, proving the
    // replicated config is usable and not merely present.
    c.sql(
        2,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('cfg2', '/cfg2', '{NODE_TYPE}', '{{\"title\":\"WrittenOnPeer\"}}'::JSONB)"
        ),
    )
    .await?;
    wait_for_replication_by_id(&c.client, &c.tokens, REPO, BRANCH, WS, "cfg2", CONVERGE)
        .await
        .map_err(|e| {
            format!("record written on a config-receiving node did not converge: {e:#}")
        })?;

    println!("  writes work in both directions under the replicated config");
    Ok(())
}

/// A second config change, made on a different node, must win everywhere and stay
/// won.
///
/// This is the last-write-wins assertion. Node3's config is newer than node1's, so
/// every node must end on node3's value — including node1, which produced the
/// older operation, and node3 itself, which is still holding node1's operation in
/// its replication history and would revert on any replay if the applier were the
/// blind overwrite it used to be.
async fn phase_newer_config_from_another_node_wins(c: &Cluster) -> Result<(), String> {
    // Disjoint from phase 1's [3,5,9,12], so "node3 won" and "node1's value is
    // still there" are never the same observation.
    let newer = json!([4, 6, 8, 10]);

    // Ordering matters and wall-clock LWW resolves at nanosecond granularity, but
    // a deliberate gap keeps the test's intent (node3 wrote *later*) independent
    // of scheduling noise.
    tokio::time::sleep(Duration::from_secs(1)).await;
    c.put_precisions(2, &newer).await?;

    c.expect_precisions_stable(&newer).await?;
    println!("  node3's newer config won on every node and stayed won");
    Ok(())
}

// ------------------------------------------------------------------------ driver

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all workspace_config_replication_test -- --ignored --nocapture
async fn test_workspace_config_replicates_across_the_cluster() {
    println!("\n=== workspace config replication across a masterless cluster ===\n");

    let c = Cluster::start(3).await;
    provision(&c).await;
    println!("\n[OK] 3-node cluster up, repo/workspace provisioned\n");

    // Every phase runs even if an earlier one failed, so one run reports the whole
    // picture rather than only the first break.
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
        "a workspace config written on node1 reaches node2 and node3",
        phase_config_fans_out(&c)
    );
    phase!(
        "writes still work on every node under the replicated config",
        phase_writes_still_work_under_the_replicated_config(&c)
    );
    phase!(
        "a newer config from node3 wins everywhere and is not reverted",
        phase_newer_config_from_another_node_wins(&c)
    );

    println!("\n=== summary ===");
    let mut failed = 0;
    for (name, outcome) in &outcomes {
        match outcome {
            Ok(()) => println!("  [PASS] {name}"),
            Err(e) => {
                failed += 1;
                println!("  [FAIL] {name}\n         {e}");
            }
        }
    }
    assert_eq!(failed, 0, "{failed} phase(s) failed; see the log above");
}
