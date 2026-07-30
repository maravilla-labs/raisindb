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

//! Cluster fixture for the spatial-config replication suite.
//!
//! Deliberately a near-copy of `spatial_cluster_test`'s fixture rather than a
//! shared abstraction: the two suites assert different things about the same
//! machinery, and a fixture factored to serve both would have to grow options for
//! every difference. What *is* shared — the process harness, the REST client and
//! the cross-node verification helpers — already lives in `cluster_test_utils`.

use crate::cluster_test_utils::{self, ClusterConfig, ClusterProcess, RestClient};

use serde_json::Value;
use std::time::Duration;

pub const TENANT: &str = "default";
pub const REPO: &str = "spatial_cfg_repl";
pub const BRANCH: &str = "main";
pub const WS: &str = "places";
pub const NODE_TYPE: &str = "geo:Place";

/// The password `cluster_test_utils::config::NodeConfig::new` bakes into every
/// generated node config.
const PASSWORD: &str = "Admin123!@#$";

/// Zurich Hauptbahnhof. Any lon/lat would do; a real place makes a failure legible.
pub const CENTER: (f64, f64) = (8.5402, 47.3779);

/// Replication plus the peer's own index write is not instantaneous; every
/// cross-node assertion polls up to this long before failing.
pub const CONVERGE: Duration = Duration::from_secs(90);

/// How long a restarted node gets to bind its HTTP listener.
///
/// Generous on purpose. A node coming back into a live cluster replays a
/// replication backlog before it serves, and these suites are routinely run
/// alongside other cluster tests on one machine; a 90-second budget produced
/// spurious failures where the node was healthy moments later.
const RESTART_HEALTH: Duration = Duration::from_secs(240);

// ----------------------------------------------------------------- SQL builders

/// The index-eligible spelling of "within `radius` metres of (`lon`, `lat`)".
///
/// `CAST(properties->>'x' AS GEOMETRY)` is the only spelling that survives
/// analysis today; the bare forms fail in the analyzer rather than the planner.
/// Copied from `spatial_cluster_test` so the two suites are comparing like with
/// like.
pub fn dwithin(lon: f64, lat: f64, radius: f64) -> String {
    format!(
        "SELECT name FROM '{WS}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({lon}, {lat}), {radius}) \
         ORDER BY name"
    )
}

/// An `INSERT` placing `id` at (`lon`, `lat`).
pub fn insert_place(id: &str, lon: f64, lat: f64) -> String {
    format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) \
         VALUES ('{id}', '/{id}', '{NODE_TYPE}', \
                 '{{\"title\":\"{id}\",\"location\":\
                    {{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}}}'::JSONB)"
    )
}

// ------------------------------------------------------------------- the cluster

pub struct Cluster {
    pub cluster: ClusterProcess,
    pub client: RestClient,
    pub tokens: Vec<String>,
}

impl Cluster {
    pub async fn start(node_count: usize) -> Self {
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

    pub fn nodes(&self) -> usize {
        self.client.base_urls.len()
    }

    pub fn url(&self, node: usize) -> &str {
        &self.client.base_urls[node]
    }

    pub fn token(&self, node: usize) -> &str {
        &self.tokens[node]
    }

    /// Run a statement on one node.
    pub async fn sql(&self, node: usize, sql: &str) -> Result<Value, String> {
        self.client
            .execute_sql(self.url(node), self.token(node), REPO, sql, vec![])
            .await
            .map_err(|e| format!("{e:#}"))
    }

    pub async fn wait_for_record(&self, node_id: &str) -> Result<(), String> {
        cluster_test_utils::wait_for_replication_by_id(
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

    /// Kill a node and start it again on the same data directory, then wait for it
    /// to serve.
    ///
    /// See `ClusterProcess::restart_node` for why this suite kills nodes rather than
    /// stopping them.
    pub async fn restart(&mut self, node: usize) -> Result<(), String> {
        self.cluster
            .restart_node(node)
            .map_err(|e| format!("{e:#}"))?;
        self.cluster
            .wait_for_node_health(node, RESTART_HEALTH)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    /// Byte length of every node log right now, so a later [`Self::log_lines_since`]
    /// can ignore everything that happened before this point.
    ///
    /// Necessary, not fastidious: creating a repository on three nodes at once
    /// makes the system workspaces (`functions`, `packages`, `raisin:system`, …)
    /// race, and they produce genuine LWW rejections during provisioning. Grepping
    /// the whole log for a rejection therefore "proves" the guard fired no matter
    /// what the phase under test did — the first version of this suite reported 19
    /// rejections, and not one of them was for the workspace it had partitioned.
    pub fn log_mark(&self) -> LogMark {
        LogMark(
            self.cluster
                .log_paths()
                .iter()
                .map(|logs| {
                    let len =
                        |p: &std::path::Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                    (len(&logs.stdout_path), len(&logs.stderr_path))
                })
                .collect(),
        )
    }

    /// Log lines written *after* `mark` that contain every string in `needles`.
    ///
    /// Used to report whether the LWW guard actually fired for the workspace under
    /// test, which the config assertions cannot distinguish from "the stale
    /// operation was never re-delivered at all".
    ///
    /// A node restarted since the mark was taken has had its log **truncated**, so
    /// the recorded offset now points past the end of a file whose every line is
    /// new. Slicing from it would silently return nothing — losing exactly the
    /// evidence this exists to collect. A file shorter than its mark is therefore
    /// read from the beginning.
    pub fn log_lines_since(&self, mark: &LogMark, needles: &[&str]) -> Vec<String> {
        let mut hits = Vec::new();
        for (idx, logs) in self.cluster.log_paths().iter().enumerate() {
            let (stdout_from, stderr_from) = mark.0.get(idx).copied().unwrap_or((0, 0));
            for (path, from) in [
                (&logs.stdout_path, stdout_from),
                (&logs.stderr_path, stderr_from),
            ] {
                let Ok(content) = std::fs::read_to_string(path) else {
                    continue;
                };
                let from = if (content.len() as u64) < from {
                    0
                } else {
                    from as usize
                };
                let tail = content.get(from..).unwrap_or("");
                for line in tail
                    .lines()
                    .filter(|l| needles.iter().all(|n| l.contains(n)))
                {
                    hits.push(format!("{}: {}", logs.node_id, line.trim()));
                }
            }
        }
        hits
    }
}

/// Opaque per-node log offsets captured by [`Cluster::log_mark`].
pub struct LogMark(Vec<(u64, u64)>);

async fn authenticate_with_retry(client: &RestClient, url: &str, idx: usize) -> String {
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=10 {
        match client.authenticate(url, TENANT, "admin", PASSWORD).await {
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
