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

//! One connection, several subscriptions in ONE workspace: ending one of them
//! must not silence the rest — over a REAL running server and the actual
//! WebSocket wire protocol.
//!
//! The registry indexes connections by workspace so an event can be fanned out
//! without walking every connection, and a connection appears in a workspace's
//! bucket ONCE however many subscriptions it holds there. Giving that entry up
//! on the first `unsubscribe` therefore took every REMAINING subscription on
//! the connection out of the fan-out with it: the filters still matched, but
//! `get_by_workspace` stopped offering the connection the workspace's events,
//! so they went silent while still appearing to be subscribed. Any client that
//! subscribes and unsubscribes as its views come and go — which is every
//! long-lived UI — went quiet on its first teardown and only recovered by
//! reconnecting.
//!
//! `raisin-transport-ws`'s own unit tests cover the registry and the fan-out in
//! isolation. This drives the wire: authenticate, subscribe twice, unsubscribe
//! one, write, and require the push for the one still open.

#[allow(unused_imports)]
use crate::helpers;
use futures_util::{SinkExt, StreamExt};
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{bootstrap_admin, http_post, http_put, PASSWORD};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const REPO: &str = "subsindex";
const BRANCH: &str = "main";
const WS: &str = "notes";
const NODE_TYPE: &str = "test:Note";
const PORT: u16 = 8144;

async fn provision(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "workspace subscription index test repo",
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
            "description": "Notes to watch",
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
                "description": "A note",
                "properties": [ { "name": "title", "type": "String" } ],
                "allowed_children": []
            },
            "commit": { "message": "Create test NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(Duration::from_millis(300)).await;
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(socket: &mut Socket, envelope: &Value) {
    let bytes = rmp_serde::to_vec_named(envelope).expect("encode");
    socket
        .send(Message::Binary(bytes.into()))
        .await
        .expect("send");
}

/// The next response envelope, skipping the greeting and any event push.
async fn reply(socket: &mut Socket) -> Value {
    next_frame(socket, |v| v.get("status").is_some())
        .await
        .expect("a response envelope")
}

/// The next event push FOR `subscription_id`, or `None` within `timeout`.
///
/// Scoped to one subscription on purpose: the server also pushes system
/// frames that carry a `subscription_id` of their own (`__permissions__` after
/// an access-control write), so "the next frame with a subscription_id" is a
/// race that can pass for the wrong reason.
async fn event_for(socket: &mut Socket, subscription_id: &str, timeout: Duration) -> Option<Value> {
    tokio::time::timeout(timeout, async {
        loop {
            let message = socket.next().await?.ok()?;
            if let Message::Binary(data) = message {
                if let Ok(value) = rmp_serde::from_slice::<Value>(&data) {
                    if value["subscription_id"] == subscription_id {
                        return Some(value);
                    }
                }
            }
        }
    })
    .await
    .ok()
    .flatten()
}

async fn next_frame(socket: &mut Socket, want: fn(&Value) -> bool) -> Option<Value> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(30), socket.next())
            .await
            .ok()??
            .ok()?;
        if let Message::Binary(data) = message {
            let value: Value = rmp_serde::from_slice(&data).ok()?;
            if want(&value) {
                return Some(value);
            }
        }
    }
}

async fn subscribe(socket: &mut Socket, request_id: &str, path: &str) -> String {
    send(
        socket,
        &json!({
            "request_id": request_id,
            "type": "subscribe",
            "context": { "tenant_id": "default", "repository": REPO, "branch": BRANCH },
            "payload": { "filters": {
                "workspace": WS,
                "path": path,
                "event_types": ["node:created", "node:updated", "node:deleted"]
            } }
        }),
    )
    .await;
    let response = reply(socket).await;
    assert_eq!(
        response["status"], "success",
        "subscribe failed: {response}"
    );
    response["result"]["subscription_id"]
        .as_str()
        .expect("subscription_id")
        .to_string()
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all subscription_workspace_index_test -- --ignored --nocapture
async fn a_remaining_subscription_still_receives_after_a_sibling_unsubscribes() {
    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    provision(&server.base_url, &token).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{PORT}/ws/{REPO}"))
            .await
            .expect("ws connect");

    send(
        &mut socket,
        &json!({
            "request_id": "auth-1",
            "type": "authenticate",
            "context": { "tenant_id": "default", "repository": REPO },
            "payload": { "username": "admin", "password": PASSWORD }
        }),
    )
    .await;
    let authenticated = reply(&mut socket).await;
    assert_eq!(
        authenticated["status"], "success",
        "authenticate failed: {authenticated}"
    );

    // Two views on one connection, both watching the same workspace.
    let keep = subscribe(&mut socket, "sub-1", "/**").await;
    let drop = subscribe(&mut socket, "sub-2", "/folder/**").await;
    assert_ne!(keep, drop, "different filters must not share an id");

    // One view closes.
    send(
        &mut socket,
        &json!({
            "request_id": "unsub-1",
            "type": "unsubscribe",
            "context": { "tenant_id": "default", "repository": REPO, "branch": BRANCH },
            "payload": { "subscription_id": drop }
        }),
    )
    .await;
    let unsubscribed = reply(&mut socket).await;
    assert_eq!(
        unsubscribed["status"], "success",
        "unsubscribe failed: {unsubscribed}"
    );

    // A write the surviving subscription matches.
    send(
        &mut socket,
        &json!({
            "request_id": "sql-1",
            "type": "sql_query",
            "context": { "tenant_id": "default", "repository": REPO, "branch": BRANCH },
            "payload": { "query": format!(
                "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
                 ('note-1','/note-1','{NODE_TYPE}','{{\"title\":\"a\"}}'::JSONB)"
            ) }
        }),
    )
    .await;
    let inserted = reply(&mut socket).await;
    assert_ne!(inserted["status"], "error", "insert failed: {inserted}");

    assert!(
        event_for(&mut socket, &keep, Duration::from_secs(10))
            .await
            .is_some(),
        "the subscription that is still open must receive the event; none \
         arrived, so ending its sibling took the connection out of the \
         workspace's fan-out"
    );

    let _ = socket.close(None).await;
}
