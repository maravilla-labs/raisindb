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

//! Reading the admin surface: health, verify, config, and the job queue.
//!
//! `health` deliberately reads the **HTTP** endpoint rather than
//! `SHOW SPATIAL INDEX HEALTH`: the two are supposed to be the same record seen
//! through two surfaces, and the HTTP one is the surface an admin console drives.
//! The SQL form is cross-checked separately.

use serde_json::{json, Value};

use super::transport::{admin_path, http_get, http_post, http_put, PROPERTY, REPO, WORKSPACE};

/// One `GET …/spatial/health` entry, flattened.
#[derive(Debug, Clone)]
pub struct Health {
    pub phase: String,
    /// What the local index was last BUILT under.
    pub indexed: Vec<usize>,
    /// Replicated intent, from the workspace record.
    pub configured: Vec<usize>,
    pub needs_rebuild: bool,
    pub live_entries: u64,
    pub distinct_nodes: u64,
    pub per_precision: Vec<(usize, u64)>,
}

impl Health {
    /// Live entries at one precision; 0 when that precision has none.
    pub fn at(&self, precision: usize) -> u64 {
        self.per_precision
            .iter()
            .find(|(p, _)| *p == precision)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    }

    /// Reality has caught up with intent.
    pub fn settled_at(&self, precisions: &[usize]) -> bool {
        self.indexed == sorted(precisions.to_vec())
            && self.configured == sorted(precisions.to_vec())
            && !self.needs_rebuild
            && self.live_entries == self.distinct_nodes * precisions.len() as u64
    }
}

impl std::fmt::Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "phase={} indexed={:?} configured={:?} needs_rebuild={} nodes={} entries={} per_precision={:?}",
            self.phase,
            self.indexed,
            self.configured,
            self.needs_rebuild,
            self.distinct_nodes,
            self.live_entries,
            self.per_precision
        )
    }
}

pub fn sorted(mut v: Vec<usize>) -> Vec<usize> {
    v.sort_unstable();
    v
}

/// `GET …/spatial/health?workspace=…&property=…`.
pub async fn health(base_url: &str, token: &str) -> Health {
    let body = http_get(
        base_url,
        &format!(
            "{}?workspace={WORKSPACE}&property={PROPERTY}",
            admin_path("health")
        ),
        token,
    )
    .await
    .expect("GET spatial/health");
    let entry = body
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| panic!("no health entry for {WORKSPACE}.{PROPERTY}: {body}"));

    let list = |key: &str| -> Vec<usize> {
        entry[key]
            .as_array()
            .map(|a| {
                sorted(
                    a.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as usize))
                        .collect(),
                )
            })
            .unwrap_or_default()
    };

    Health {
        phase: entry["phase"].as_str().unwrap_or_default().to_string(),
        indexed: list("indexed_precisions"),
        configured: list("configured_precisions"),
        needs_rebuild: entry["needs_rebuild"].as_bool().unwrap_or(false),
        live_entries: entry["live_entries"].as_u64().unwrap_or(0),
        distinct_nodes: entry["distinct_nodes"].as_u64().unwrap_or(0),
        per_precision: entry["live_per_precision"]
            .as_array()
            .map(|a| {
                let mut v: Vec<(usize, u64)> = a
                    .iter()
                    .filter_map(|pair| {
                        Some((pair.get(0)?.as_u64()? as usize, pair.get(1)?.as_u64()?))
                    })
                    .collect();
                v.sort_unstable();
                v
            })
            .unwrap_or_default(),
    }
}

/// `POST …/spatial/verify` → `(status, detail)`.
pub async fn verify(base_url: &str, token: &str) -> (String, String) {
    let body = http_post(
        base_url,
        &admin_path("verify"),
        token,
        json!({ "workspace": WORKSPACE, "property": PROPERTY }),
    )
    .await
    .expect("POST spatial/verify");
    let entry = body
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| panic!("no verify entry: {body}"));
    (
        entry["status"].as_str().unwrap_or_default().to_string(),
        entry["detail"].as_str().unwrap_or_default().to_string(),
    )
}

/// `PUT …/spatial/config`, returning the whole response body.
pub async fn put_config(base_url: &str, token: &str, precisions: &[usize]) -> Value {
    http_put(
        base_url,
        &admin_path("config"),
        token,
        json!({
            "workspace": WORKSPACE,
            "property": PROPERTY,
            "precisions": precisions,
        }),
    )
    .await
    .expect("PUT spatial/config")
}

/// One job by id, from the management job API. `Err` on a non-2xx (404 included).
pub async fn job_info(base_url: &str, token: &str, job_id: &str) -> Result<Value, String> {
    http_get(base_url, &format!("/management/jobs/{job_id}/info"), token)
        .await
        .map(|v| v["data"].clone())
}

/// Every job the management API lists for this repo.
pub async fn list_jobs(base_url: &str, token: &str) -> Vec<Value> {
    http_get(base_url, &format!("/management/jobs?repo={REPO}"), token)
        .await
        .map(|v| v["data"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
}

/// Poll the health endpoint until `predicate` holds.
///
/// Polled rather than slept: reindexing and reconciliation are asynchronous, so a
/// fixed sleep is either flaky or needlessly slow.
pub async fn await_health(
    base_url: &str,
    token: &str,
    what: &str,
    predicate: impl Fn(&Health) -> bool,
) -> Health {
    for attempt in 0..80 {
        let h = health(base_url, token).await;
        if predicate(&h) {
            println!("[OK] {what} after {} poll(s): {h}", attempt + 1);
            return h;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let h = health(base_url, token).await;
    panic!("{what}: timed out; last health = {h}");
}
