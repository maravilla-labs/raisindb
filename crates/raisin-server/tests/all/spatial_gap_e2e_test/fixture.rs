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

//! Repo/workspace/nodetype provisioning and the two write paths.
//!
//! Both write paths are exercised on purpose: SQL DML and the REST node API reach
//! the spatial index through different code, and the DML one is where a
//! `PropertyValue::Geometry` failed to be produced at all until recently.

use serde_json::{json, Value};

use super::transport::{http_post, http_put, sql, BRANCH, PROPERTY, REPO, TENANT, WORKSPACE};

/// Authenticate as the tenant admin and clear `must_change_password`.
pub async fn bootstrap_admin(base_url: &str) -> String {
    let token =
        crate::helpers::multi_node::authenticate(base_url, TENANT, "admin", "Admin12345!@#")
            .await
            .expect("authenticate");
    let client = reqwest::Client::new();
    let profile: Value = client
        .get(format!("{base_url}/api/raisindb/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap().to_string();
    let _ = client
        .put(format!(
            "{base_url}/api/raisindb/sys/{TENANT}/users/{user_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await;
    crate::helpers::multi_node::authenticate(base_url, TENANT, "admin", "Admin12345!@#")
        .await
        .expect("re-authenticate")
}

/// Repository, workspace and a node type carrying one geometry property.
pub async fn provision(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "SRID normalisation + precision policy, end to end",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WORKSPACE}"),
        token,
        json!({
            "name": WORKSPACE,
            "description": "sites in assorted CRS",
            // `raisin:Folder` must be allowed as a root type: workspace creation
            // materialises a root folder node and is rejected without it.
            "allowed_node_types": ["gap:Site", "raisin:Folder"],
            "allowed_root_node_types": ["gap:Site", "raisin:Folder"],
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
                "name": "gap:Site",
                "description": "A site with a geometry in some CRS",
                "properties": [
                    { "name": "title", "type": "String", "required": true },
                    { "name": PROPERTY, "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "create gap:Site", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}

fn point(x: f64, y: f64, srid: Option<u32>) -> Value {
    match srid {
        Some(s) => json!({ "type": "Point", "coordinates": [x, y], "srid": s }),
        None => json!({ "type": "Point", "coordinates": [x, y] }),
    }
}

/// Insert over **SQL DML**.
pub async fn insert_via_sql(
    base_url: &str,
    token: &str,
    id: &str,
    x: f64,
    y: f64,
    srid: Option<u32>,
) -> Result<Value, String> {
    let props = json!({ "title": id, PROPERTY: point(x, y, srid) })
        .to_string()
        .replace('\'', "''");
    sql(
        base_url,
        token,
        &format!(
            "INSERT INTO '{WORKSPACE}' (id, name, node_type, path, properties) VALUES \
             ('{id}', '{id}', 'gap:Site', '/{id}', '{props}'::jsonb)"
        ),
    )
    .await
}

/// Insert over the **REST node API**.
pub async fn insert_via_rest(
    base_url: &str,
    token: &str,
    id: &str,
    x: f64,
    y: f64,
    srid: Option<u32>,
) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/"),
        token,
        json!({
            "node": {
                "id": id,
                "name": id,
                "node_type": "gap:Site",
                "properties": { "title": id, PROPERTY: point(x, y, srid) }
            }
        }),
    )
    .await
}

/// Every node name in the workspace, sorted — used to prove a rejected write
/// stored nothing.
pub async fn all_site_names(base_url: &str, token: &str) -> Vec<String> {
    let r = sql(
        base_url,
        token,
        &format!("SELECT name FROM '{WORKSPACE}' WHERE node_type = 'gap:Site' ORDER BY name"),
    )
    .await
    .expect("SELECT name");
    super::transport::names(&r)
}
