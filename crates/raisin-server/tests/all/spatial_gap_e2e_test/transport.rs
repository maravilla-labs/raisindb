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

//! HTTP transport for the suite.
//!
//! Everything in this test goes over the wire to a real server process; nothing
//! calls into the crates under test directly. That is deliberate — every bug this
//! suite covers lived in the wiring *between* layers, so a helper that
//! short-circuited the transport would prove nothing.

use reqwest::Client;
use serde_json::{json, Value};

pub const TENANT: &str = "default";
pub const REPO: &str = "spatial_gap_e2e";
pub const BRANCH: &str = "main";
pub const WORKSPACE: &str = "sites";
pub const PROPERTY: &str = "geom";

/// `INDEX_PRECISIONS_DEFAULT` — what a workspace indexes at with no admin action.
pub const DEFAULT_PRECISIONS: [usize; 8] = [2, 4, 6, 7, 8, 9, 10, 11];

/// The admin base path for this repo's spatial surface.
pub fn admin_path(suffix: &str) -> String {
    format!("/api/admin/management/database/{TENANT}/{REPO}/spatial/{suffix}")
}

/// `Err` carries `"{status}: {body}"`, so an assertion can inspect both.
pub async fn http_post(
    base_url: &str,
    path: &str,
    token: &str,
    body: Value,
) -> Result<Value, String> {
    let response = Client::new()
        .post(format!("{base_url}{path}"))
        .bearer_auth(token)
        .header("x-tenant-id", TENANT)
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

pub async fn http_put(
    base_url: &str,
    path: &str,
    token: &str,
    body: Value,
) -> Result<Value, String> {
    let response = Client::new()
        .put(format!("{base_url}{path}"))
        .bearer_auth(token)
        .header("x-tenant-id", TENANT)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{status}: {text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

pub async fn http_get(base_url: &str, path: &str, token: &str) -> Result<Value, String> {
    let response = Client::new()
        .get(format!("{base_url}{path}"))
        .bearer_auth(token)
        .header("x-tenant-id", TENANT)
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

pub async fn sql(base_url: &str, token: &str, statement: &str) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{REPO}"),
        token,
        json!({ "sql": statement, "params": [] }),
    )
    .await
}

pub fn rows(response: &Value) -> Vec<Value> {
    response["rows"].as_array().cloned().unwrap_or_default()
}

/// The `name` column of every row, in the order the server returned them.
pub fn names(response: &Value) -> Vec<String> {
    rows(response)
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_string))
        .collect()
}
