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

//! Live-server harness for the ST_* conformance suite.
//!
//! Same shape as `geospatial_test.rs`: real `raisin-server` process, real HTTP,
//! real stored nodes, SQL over `POST /api/sql/{repo}`.

use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

use super::coverage::Coverage;
use super::fixtures;

pub const REPO: &str = "st_conf";
pub const BRANCH: &str = "main";
pub const WORKSPACE: &str = "shapes";
pub const NODE_TYPE: &str = "conf:Shape";

/// A running server plus the token and the coverage ledger.
pub struct Ctx {
    pub server: ServerHandle,
    pub token: String,
    pub client: Client,
    pub cov: Coverage,
    /// Assertion failures collected rather than panicked on, so ONE run reports
    /// every broken function instead of stopping at the first. A conformance
    /// suite that dies on failure #1 hides failures #2..#N, which is precisely
    /// the information needed to judge release readiness.
    pub failures: Vec<String>,
    /// Defects found that are NOT in the `ST_*` library — write-path validation,
    /// argument-range checks, and so on.
    ///
    /// Kept apart from `failures` on purpose. They are printed loudly and
    /// reported, but they do not turn this suite red, because a red run here
    /// should mean "an ST_* function computes the wrong answer". Folding an
    /// unimplemented write-path check into that verdict would make the suite
    /// permanently red and therefore ignored, which is how a gate stops working.
    pub product_gaps: Vec<String>,
}

impl Ctx {
    /// Boot a server, authenticate, and create repo / workspace / nodetype /
    /// fixture nodes.
    pub async fn start(port: u16) -> Ctx {
        let config = ServerConfig::new(port);
        let server = ServerHandle::start(config)
            .await
            .expect("failed to start raisin-server");

        // The admin user is created asynchronously after the port opens.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
            .await
            .expect("initial authenticate");

        let client = Client::new();

        // Clear must_change_password, then re-auth for a clean token.
        let profile: Value = client
            .get(format!("{}/api/raisindb/me", server.base_url))
            .bearer_auth(&token)
            .send()
            .await
            .expect("me")
            .json()
            .await
            .expect("me json");
        let user_id = profile["user_id"].as_str().expect("user_id").to_string();
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
            .expect("re-authenticate");

        let mut ctx = Ctx {
            server,
            token,
            client,
            cov: Coverage::new(),
            failures: Vec::new(),
            product_gaps: Vec::new(),
        };

        ctx.provision().await;
        fixtures::insert_all(&mut ctx).await;
        ctx
    }

    async fn provision(&self) {
        self.post(
            "/api/repositories",
            json!({
                "repo_id": REPO,
                "description": "ST_* conformance",
                "default_branch": BRANCH,
            }),
        )
        .await
        .expect("create repository");

        self.put(
            &format!("/api/workspaces/{REPO}/{WORKSPACE}"),
            json!({
                "name": WORKSPACE,
                "description": "geometry fixtures",
                // raisin:Folder must be allowed as a root type or workspace
                // provisioning fails while creating its initial structure.
                "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
                "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
                "depends_on": [],
                "config": { "default_branch": BRANCH, "node_type_pins": {} },
            }),
        )
        .await
        .expect("create workspace");

        self.post(
            &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
            json!({
                "node_type": {
                    "name": NODE_TYPE,
                    "description": "holds one geometry under 'g'",
                    "properties": [
                        { "name": "label", "type": "String", "required": true },
                        { "name": "kind",  "type": "String" },
                        // Declared Geometry: this is what makes the write path
                        // fail loudly on malformed GeoJSON instead of silently
                        // storing an unindexed Object.
                        { "name": "g",     "type": "Geometry" },
                    ],
                    "allowed_children": [],
                },
                "commit": { "message": "conformance nodetype", "actor": "test" },
            }),
        )
        .await
        .expect("create nodetype");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let resp = self
            .client
            .post(format!("{}{}", self.server.base_url, path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|_| text)
    }

    pub async fn put(&self, path: &str, body: Value) -> Result<(), String> {
        let resp = self
            .client
            .put(format!("{}{}", self.server.base_url, path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("{status}: {text}"));
        }
        Ok(())
    }

    /// Run SQL, returning the row array on success.
    pub async fn sql(&self, sql: &str) -> Result<Vec<Value>, String> {
        let out = self
            .post(
                &format!("/api/sql/{REPO}"),
                json!({ "sql": sql, "params": [] }),
            )
            .await?;
        Ok(out["rows"].as_array().cloned().unwrap_or_default())
    }

    /// Evaluate a scalar expression and return the single value.
    ///
    /// `SELECT <expr> AS r` with no FROM — the pure-computation path.
    pub async fn scalar(&self, expr: &str) -> Result<Value, String> {
        let rows = self.sql(&format!("SELECT {expr} AS r")).await?;
        match rows.first() {
            // A NULL result is omitted from the row map by the projection
            // executor, so an absent key and an explicit null are the same thing.
            Some(row) => Ok(row.get("r").cloned().unwrap_or(Value::Null)),
            None => Err("no rows returned".to_string()),
        }
    }
}

/// A geometry literal usable as an ST_* argument: `ST_GEOMFROMGEOJSON('...')`.
pub fn g(geojson: &str) -> String {
    format!("ST_GEOMFROMGEOJSON('{geojson}')")
}

mod assertions;
