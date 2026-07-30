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

//! Client for the admin spatial-config surface, plus the convergence predicates
//! the phases assert with.
//!
//! `PUT /api/admin/management/database/{tenant}/{repo}/spatial/config` is the
//! endpoint under test — **not** `PUT /api/workspaces/{repo}/{ws}/config`. They
//! reach the same replicated record by different routes: the admin endpoint
//! merges into the existing `SpatialWorkspaceSchema` rather than replacing the
//! whole `WorkspaceConfig`, and additionally queues a local rebuild. A suite that
//! exercised only the workspace endpoint would leave the surface an operator
//! actually uses untested.

use super::fixture::{Cluster, CONVERGE, REPO, TENANT, WS};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

fn config_path() -> String {
    format!("/api/admin/management/database/{TENANT}/{REPO}/spatial/config")
}

/// Every request here carries a deadline.
///
/// Not defensive boilerplate — this suite kills and freezes nodes on purpose, and
/// `reqwest` has no default timeout. A node left wedged by a bug therefore hangs
/// the request forever, and the whole suite hangs *silently* inside a phase
/// instead of failing it: an early negative-control run sat for twenty minutes
/// with node1 stopped and reported nothing. A bounded request turns that into a
/// phase failure that names the node.
fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("reqwest client")
}

/// `PUT …/spatial/config`, setting the workspace-default precision set.
///
/// Returns the endpoint's own view of the resulting policy, which carries
/// `rebuild_job_id` when the change made the local index stale.
pub async fn put_precisions(
    c: &Cluster,
    node: usize,
    precisions: &[usize],
) -> Result<Value, String> {
    let response = client()
        .put(format!("{}{}", c.url(node), config_path()))
        .bearer_auth(c.token(node))
        .json(&json!({ "workspace": WS, "precisions": precisions }))
        .send()
        .await
        .map_err(|e| format!("spatial config PUT on node{}: {e}", node + 1))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "spatial config PUT on node{} returned {status}: {text}",
            node + 1
        ));
    }
    serde_json::from_str(&text).map_err(|e| {
        format!(
            "spatial config PUT on node{} is not JSON ({e}): {text}",
            node + 1
        )
    })
}

/// `GET …/spatial/config`, returning the workspace-default precision set.
///
/// The endpoint answers with one entry per configured scope; the workspace
/// default is reported under property `*`.
pub async fn get_precisions(c: &Cluster, node: usize) -> Result<Vec<usize>, String> {
    let response = client()
        .get(format!("{}{}?workspace={WS}", c.url(node), config_path()))
        .bearer_auth(c.token(node))
        .send()
        .await
        .map_err(|e| format!("spatial config GET on node{}: {e}", node + 1))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "spatial config GET on node{} returned {status}: {text}",
            node + 1
        ));
    }
    let entries: Vec<Value> = serde_json::from_str(&text).map_err(|e| {
        format!(
            "spatial config GET on node{} is not JSON ({e}): {text}",
            node + 1
        )
    })?;

    let entry = entries
        .iter()
        .find(|e| e["property"] == "*")
        .ok_or_else(|| format!("node{} reported no '*' scope: {text}", node + 1))?;

    Ok(normalised(&entry["precisions"]))
}

/// A precision list as an ascending `Vec<usize>`.
///
/// The server stores precisions sorted descending (`sorted_precisions`), so
/// comparing raw arrays would make the test sensitive to a storage-order detail
/// that carries no meaning. Set equality is what "the config arrived" means.
fn normalised(value: &Value) -> Vec<usize> {
    let mut out: Vec<usize> = value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64().map(|n| n as usize))
                .collect()
        })
        .unwrap_or_default();
    out.sort_unstable();
    out
}

/// Ascending copy of `want`, for comparing against [`get_precisions`].
pub fn ascending(want: &[usize]) -> Vec<usize> {
    let mut out = want.to_vec();
    out.sort_unstable();
    out
}

/// Poll every node until all of them report `want`, or give up.
///
/// Every node is required, not "some node other than the writer": a change that
/// reaches one peer and not another is exactly the heterogeneous state the
/// fan-out exists to prevent, and it is also what a one-directional bug looks
/// like.
pub async fn expect_everywhere(c: &Cluster, want: &[usize]) -> Result<(), String> {
    let want = ascending(want);
    let deadline = tokio::time::Instant::now() + CONVERGE;
    loop {
        let mut pending = Vec::new();
        for node in 0..c.nodes() {
            match get_precisions(c, node).await {
                Ok(got) if got == want => {}
                Ok(got) => pending.push(format!("node{} has {got:?}", node + 1)),
                Err(e) => pending.push(format!("node{} unreadable: {e}", node + 1)),
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "spatial config did not reach every node within {}s (wanted {want:?}): {}",
                CONVERGE.as_secs(),
                pending.join("; ")
            ));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Assert every node reports `want` and *keeps* reporting it.
///
/// A single converged sample cannot tell a settled cluster from one that is
/// midway through being clobbered by a late older operation — which is the entire
/// failure mode the LWW guard exists to prevent — so the state is re-read after a
/// settle window.
pub async fn expect_stable(c: &Cluster, want: &[usize], settle: Duration) -> Result<(), String> {
    expect_everywhere(c, want).await?;
    tokio::time::sleep(settle).await;

    let want = ascending(want);
    for node in 0..c.nodes() {
        let got = get_precisions(c, node).await?;
        if got != want {
            return Err(format!(
                "node{} reverted after converging: expected {want:?}, got {got:?} — \
                 an older UpdateWorkspace clobbered a newer one",
                node + 1
            ));
        }
    }
    Ok(())
}
