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

//! The client every section drives the server through.
//!
//! Everything goes over the wire — SQL over `POST /api/sql/{repo}`, admin over
//! `/api/admin/management/database/…`. Nothing calls into the crates under test
//! directly; the bugs this suite covers all lived in the wiring between layers.

use super::{REPO, TENANT, WS};
use crate::helpers::sql_geo::{bootstrap_admin, sql_http};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

pub struct Ctx {
    pub base_url: String,
    pub token: String,
}

impl Ctx {
    pub async fn bootstrap(base_url: &str) -> Self {
        let token = bootstrap_admin(base_url).await;
        Self {
            base_url: base_url.to_string(),
            token,
        }
    }

    // ------------------------------------------------------------------- SQL

    pub async fn run(&self, sql: &str) -> Value {
        sql_http(&self.base_url, &self.token, REPO, sql)
            .await
            .unwrap_or_else(|e| panic!("SQL failed: {e}\n  query: {sql}"))
    }

    pub async fn try_run(&self, sql: &str) -> Result<Value, String> {
        sql_http(&self.base_url, &self.token, REPO, sql).await
    }

    /// The `name` column of every row, in the order the server returned them.
    pub async fn names(&self, sql: &str) -> Vec<String> {
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

    /// Poll until the row set matches, then assert. Writes land in the write
    /// batch, but the HTTP round trip that created them still has to complete, so
    /// a bare read immediately after a write is a race rather than a proof.
    pub async fn expect_names(&self, label: &str, sql: &str, want: &[&str]) {
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

    pub async fn scalar(&self, sql: &str, column: &str) -> Option<f64> {
        let result = self.run(sql).await;
        result["rows"][0][column].as_f64()
    }

    /// The physical plan text for a query.
    ///
    /// `EXPLAIN` renders under `explain_plan` on this transport, with the
    /// `QUERY PLAN` column as the fallback spelling.
    pub async fn explain(&self, sql: &str) -> String {
        let plan = self.run(&format!("EXPLAIN {sql}")).await;
        plan["explain_plan"]
            .as_str()
            .map(str::to_string)
            .or_else(|| plan["rows"][0]["QUERY PLAN"].as_str().map(str::to_string))
            .unwrap_or_else(|| plan.to_string())
    }

    /// Assert the plan is served by the spatial index.
    ///
    /// Correct rows do NOT prove this: a full scan re-applies the predicate per
    /// row and answers correctly too. Before this pass the nested case was
    /// *stored, healthy-looking and invisible*, so the plan is the assertion that
    /// distinguishes "the index knows about the nested path" from "the scan
    /// rescued us".
    pub async fn expect_index_backed(&self, label: &str, sql: &str) {
        let plan = self.explain(sql).await;
        assert!(
            plan.contains("SpatialDistanceScan"),
            "{label}: expected a SpatialDistanceScan — a plan without one means the \
             nested path never reached the index and the rows came from a full \
             scan\n  query: {sql}\n{plan}"
        );
        println!("[PASS] {label}: plan is a SpatialDistanceScan");
    }

    // ----------------------------------------------------------------- admin

    fn admin_path(suffix: &str) -> String {
        format!("/api/admin/management/database/{TENANT}/{REPO}/spatial/{suffix}")
    }

    pub async fn admin_get(&self, path_and_query: &str) -> Value {
        let response = Client::new()
            .get(format!("{}{path_and_query}", self.base_url))
            .bearer_auth(&self.token)
            .header("x-tenant-id", TENANT)
            .send()
            .await
            .expect("admin GET");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "admin GET {path_and_query}: {status} {text}"
        );
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    pub async fn admin_send(&self, method: reqwest::Method, suffix: &str, body: Value) -> Value {
        let response = Client::new()
            .request(
                method,
                format!("{}{}", self.base_url, Self::admin_path(suffix)),
            )
            .bearer_auth(&self.token)
            .header("x-tenant-id", TENANT)
            .json(&body)
            .send()
            .await
            .expect("admin request");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        assert!(status.is_success(), "admin {suffix}: {status} {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    /// `GET …/spatial/health?workspace=…&property=…` for one property path.
    pub async fn health(&self, property: &str) -> Health {
        let encoded = urlencode(property);
        let body = self
            .admin_get(&format!(
                "{}?workspace={WS}&property={encoded}",
                Self::admin_path("health")
            ))
            .await;
        let entry = body
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_else(|| panic!("no health entry for {WS}.{property}: {body}"));
        Health::from(&entry)
    }

    /// `PUT …/spatial/config`, returning the whole response body.
    pub async fn put_config(&self, property: &str, precisions: &[usize]) -> Value {
        self.admin_send(
            reqwest::Method::PUT,
            "config",
            json!({ "workspace": WS, "property": property, "precisions": precisions }),
        )
        .await
    }

    /// `POST …/spatial/rebuild`; `None` rebuilds every geometry property.
    pub async fn rebuild(&self, property: Option<&str>) -> String {
        let body = match property {
            Some(p) => json!({ "workspace": WS, "property": p }),
            None => json!({ "workspace": WS }),
        };
        let response = self
            .admin_send(reqwest::Method::POST, "rebuild", body)
            .await;
        response["job_id"]
            .as_str()
            .unwrap_or_else(|| panic!("no job_id in rebuild response: {response}"))
            .to_string()
    }

    /// Poll one property's health until `predicate` holds, or fail with the last
    /// reading. Rebuilds are asynchronous jobs, so a fixed sleep is either flaky
    /// or needlessly slow.
    pub async fn await_health(
        &self,
        property: &str,
        what: &str,
        predicate: impl Fn(&Health) -> bool,
    ) -> Health {
        let mut last = self.health(property).await;
        for attempt in 0..80 {
            if predicate(&last) {
                println!("[OK] {what} after {} poll(s): {last}", attempt + 1);
                return last;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            last = self.health(property).await;
        }
        panic!("{what} never happened for '{property}'; last health = {last}");
    }
}

/// One flattened `GET …/spatial/health` entry.
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
    fn from(entry: &Value) -> Self {
        let list = |key: &str| -> Vec<usize> {
            let mut v: Vec<usize> = entry[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as usize))
                        .collect()
                })
                .unwrap_or_default();
            v.sort_unstable();
            v
        };
        Self {
            phase: entry["phase"].as_str().unwrap_or_default().to_string(),
            indexed: list("indexed_precisions"),
            configured: list("configured_precisions"),
            needs_rebuild: entry["needs_rebuild"].as_bool().unwrap_or(false),
            live_entries: entry["live_entries"].as_u64().unwrap_or(0),
            distinct_nodes: entry["distinct_nodes"].as_u64().unwrap_or(0),
            per_precision: {
                let mut v: Vec<(usize, u64)> = entry["live_per_precision"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|pair| {
                                Some((pair.get(0)?.as_u64()? as usize, pair.get(1)?.as_u64()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                v.sort_unstable();
                v
            },
        }
    }

    /// Live entries at one precision; 0 when that precision has none.
    pub fn at(&self, precision: usize) -> u64 {
        self.per_precision
            .iter()
            .find(|(p, _)| *p == precision)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    }

    /// Reality has caught up with intent for this precision set.
    pub fn settled_at(&self, precisions: &[usize]) -> bool {
        let mut want = precisions.to_vec();
        want.sort_unstable();
        self.indexed == want
            && self.configured == want
            && !self.needs_rebuild
            && self.live_entries == self.distinct_nodes * want.len() as u64
    }
}

impl std::fmt::Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "phase={} indexed={:?} configured={:?} needs_rebuild={} nodes={} entries={} \
             per_precision={:?}",
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

/// Percent-encode the few characters a property path can contain that a query
/// string cares about. Dots and digits pass through untouched, which is the
/// whole point — the path in the URL is the path in the index key.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '[' => "%5B".to_string(),
            ']' => "%5D".to_string(),
            _ => c.to_string(),
        })
        .collect()
}
