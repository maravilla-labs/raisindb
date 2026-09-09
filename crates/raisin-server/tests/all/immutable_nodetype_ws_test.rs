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

//! `NodeType.immutable` enforced end-to-end over a REAL running server and
//! the actual WebSocket wire protocol (MessagePack over binary frames) — the
//! transport a browser client uses, as opposed to the in-process
//! `QueryEngine` coverage in `raisin-sql-execution`'s
//! `immutable_nodetype_dml_test.rs` or the direct transaction-layer coverage
//! in `raisin-rocksdb`'s `immutable_nodetype_test.rs`. All three write paths
//! funnel into the same `crate::immutability::reject_if_immutable` check
//! (see CLAUDE.md's `immutable` and `versionable` NodeType flags section),
//! so this proves the wire-level path reaches it too.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{bootstrap_admin, http_post, http_put, sql_ws};
use serde_json::json;
use std::time::Duration;

const REPO: &str = "immutablews";
const BRANCH: &str = "main";
const WS: &str = "ledger";
const NODE_TYPE: &str = "test:LedgerEntry";
const PORT: u16 = 8140;

/// Same shape as `helpers::sql_geo::provision`, but with `immutable: true` on
/// the NodeType — `provision` itself has no knob for that.
async fn provision_immutable(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "immutable-over-WS test repo",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WS}"),
        token,
        json!({
            "name": WS,
            "description": "Ledger entries that must never change",
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
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
                "name": NODE_TYPE,
                "description": "An append-only ledger entry",
                "properties": [
                    { "name": "title", "type": "String" }
                ],
                "allowed_children": [],
                "immutable": true
            },
            "commit": { "message": "Create immutable test NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all immutable_nodetype_ws_test -- --ignored --nocapture
async fn ws_sql_update_rejected_on_immutable_nodetype() {
    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    provision_immutable(&server.base_url, &token).await;

    // INSERT (create) over WS succeeds — immutability only blocks a
    // subsequent property change.
    sql_ws(
        PORT,
        REPO,
        BRANCH,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
             ('entry-1','/entry-1','{NODE_TYPE}','{{\"title\":\"a\"}}'::JSONB)"
        ),
    )
    .await
    .unwrap_or_else(|e| panic!("WS insert failed: {e}"));

    // UPDATE that changes `properties`, sent over the real WS wire protocol,
    // must be rejected.
    let err = sql_ws(
        PORT,
        REPO,
        BRANCH,
        &format!(
            "UPDATE '{WS}' SET properties = '{{\"title\":\"b\"}}'::jsonb WHERE path = '/entry-1'"
        ),
    )
    .await
    .expect_err("immutable nodetype must reject a property-changing UPDATE over WebSocket");
    assert!(
        err.contains("immutable"),
        "expected an immutability rejection, got: {err}"
    );
}
