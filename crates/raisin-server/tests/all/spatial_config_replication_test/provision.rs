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

//! One-time provisioning of the repository, workspace and NodeType.
//!
//! Split from `fixture.rs` purely for size; it is used once, from the test
//! driver, before any phase runs.

use super::fixture::{Cluster, BRANCH, NODE_TYPE, REPO, WS};
use crate::cluster_test_utils;
use crate::helpers::sql_geo::{http_post, http_put};
use serde_json::json;
use std::time::Duration;

/// Create the repository, workspace and NodeType.
///
/// Repository and workspace are created on **every** node, tolerating "already
/// exists": whether workspace *creation* replicates is not what this suite
/// measures, and a fixture that depended on it would be depending on the thing
/// under test. The NodeType is created once and asserted to replicate, because
/// the spatial write path resolves its policy from schema.
pub async fn provision(c: &Cluster) {
    for node in 0..c.nodes() {
        tolerate_exists(
            http_post(
                c.url(node),
                "/api/repositories",
                c.token(node),
                json!({
                    "repo_id": REPO,
                    "description": "Spatial config replication test repo",
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
            "commit": { "message": "Create geo:Place NodeType", "actor": "spatial-config-test" }
        }),
    )
    .await
    .expect("create nodetype on node1");

    cluster_test_utils::wait_for_nodetype_replication(
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
