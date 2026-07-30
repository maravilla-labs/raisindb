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

//! Shared harness for the SQL-write / transport-parity geospatial suites.
//!
//! Three transports must be driven with the *same* SQL, so the client for each
//! lives here rather than being reinvented per module:
//!
//! * **HTTP** — `POST /api/sql/{repo}`, the shape `geospatial_test.rs` uses.
//! * **WebSocket** — `sql_query` requests. The protocol is **MessagePack over
//!   Binary frames**, not JSON text: `handler/socket.rs` decodes only
//!   `Message::Binary` with `rmp_serde::from_slice` and logs a text frame as
//!   "unexpected", so a JSON-text client hangs instead of erroring.
//! * **PGWire** — a real `tokio-postgres` client. Authentication is
//!   `user = tenant_id`, `dbname = repository`, `password = <API key>`
//!   (`auth/handler.rs`), and the key's owner must additionally carry the
//!   `pgwire_access` flag, which is `false` by default
//!   (`AdminAccessFlags::default`).

use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

// --------------------------------------------------------------------- HTTP

pub async fn http_post(
    base_url: &str,
    path: &str,
    token: &str,
    body: Value,
) -> Result<Value, String> {
    let response = Client::new()
        .post(format!("{base_url}{path}"))
        .bearer_auth(token)
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

pub async fn http_put(base_url: &str, path: &str, token: &str, body: Value) -> Result<(), String> {
    let response = Client::new()
        .put(format!("{base_url}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(format!(
            "{status}: {}",
            response.text().await.unwrap_or_default()
        ));
    }
    Ok(())
}

/// Run SQL over HTTP against `repo`.
pub async fn sql_http(
    base_url: &str,
    token: &str,
    repo: &str,
    query: &str,
) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{repo}"),
        token,
        json!({ "sql": query, "params": [] }),
    )
    .await
}

// ---------------------------------------------------------------- bootstrap

/// Authenticate as the initial admin and clear `must_change_password`.
///
/// The admin user is also granted `pgwire_access`, which is **off** in
/// `AdminAccessFlags::default()` — without it every pgwire startup is rejected
/// with "User does not have pgwire access".
/// The `user_id` of the authenticated admin — the same value the admin JWT carries
/// in its `sub` claim, which is what `SET app.user` resolves an identity by.
pub async fn admin_user_id(base_url: &str, token: &str) -> String {
    let profile = Client::new()
        .get(format!("{base_url}/api/raisindb/me"))
        .bearer_auth(token)
        .send()
        .await
        .expect("profile request")
        .json::<Value>()
        .await
        .expect("profile json");
    profile["user_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no user_id in profile: {profile}"))
        .to_string()
}

pub async fn bootstrap_admin(base_url: &str) -> String {
    let token = crate::helpers::multi_node::authenticate(base_url, "default", "admin", PASSWORD)
        .await
        .expect("authenticate");

    let client = Client::new();
    let profile = client
        .get(format!("{base_url}/api/raisindb/me"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("profile request")
        .json::<Value>()
        .await
        .expect("profile json");
    let user_id = profile["user_id"].as_str().unwrap_or_default().to_string();

    // Best effort: this route does not exist on every build, and a token issued
    // while `must_change_password` is set is still accepted by the HTTP API.
    let _ = client
        .put(format!(
            "{base_url}/api/raisindb/sys/default/users/{user_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await;

    // `access_flags` is replaced wholesale, so every flag must be restated.
    let flags = json!({
        "access_flags": {
            "console_login": true,
            "cli_access": true,
            "api_access": true,
            "pgwire_access": true,
            "can_impersonate": true
        },
        "must_change_password": false
    });
    if let Err(e) = http_put(
        base_url,
        "/api/raisindb/sys/default/admin-users/admin",
        &token,
        flags,
    )
    .await
    {
        panic!("granting pgwire access to the admin user failed: {e}");
    }

    crate::helpers::multi_node::authenticate(base_url, "default", "admin", PASSWORD)
        .await
        .expect("re-authenticate")
}

pub const PASSWORD: &str = "Admin12345!@#";

/// Mint an API key for the authenticated admin; the raw token is the pgwire
/// password.
pub async fn create_api_key(base_url: &str, token: &str) -> String {
    let body = http_post(
        base_url,
        "/api/raisindb/me/api-keys",
        token,
        json!({ "name": "pgwire-parity-test" }),
    )
    .await
    .expect("create api key");
    body["token"]
        .as_str()
        .unwrap_or_else(|| panic!("no raw token in api-key response: {body}"))
        .to_string()
}

/// Create the repository, workspace and NodeType the suites share.
pub async fn provision(
    base_url: &str,
    token: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": repo,
            "description": "SQL geospatial write-path test repo",
            "default_branch": branch
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{repo}/{workspace}"),
        token,
        json!({
            "name": workspace,
            "description": "Shops with geometry written over SQL",
            // `raisin:Folder` must be allowed: creating a workspace materialises a
            // root folder node, so a workspace permitting only its own type is
            // rejected at creation time.
            "allowed_node_types": [node_type, "raisin:Folder"],
            "allowed_root_node_types": [node_type, "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": branch, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create workspace");

    http_post(
        base_url,
        &format!("/api/management/{repo}/{branch}/nodetypes"),
        token,
        json!({
            "node_type": {
                "name": node_type,
                "description": "A shop with a location",
                "properties": [
                    { "name": "title", "type": "String" },
                    { "name": "floor", "type": "String" },
                    { "name": "location", "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create test NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(Duration::from_millis(300)).await;
}

// ----------------------------------------------------------------- WebSocket

/// Run SQL over the WebSocket transport.
///
/// Authenticates with username/password rather than the JWT the caller already
/// holds: the token issued by `POST /api/raisindb/sys/{tenant}/auth` — accepted by
/// every HTTP endpoint — is **rejected** by the WebSocket handler with
/// "Invalid user token: missing field `email`". That divergence is reported as a
/// finding, not worked around silently.
pub async fn sql_ws(port: u16, repo: &str, branch: &str, query: &str) -> Result<Value, String> {
    // The repository must be in the URL: a bare `/ws` scopes the connection to the
    // tenant's default repository and every request naming another one is rejected
    // with REPOSITORY_SCOPE_MISMATCH.
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws/{repo}"))
            .await
            .map_err(|e| format!("ws connect: {e}"))?;

    let auth = json!({
        "request_id": "auth-1",
        "type": "authenticate",
        "context": { "tenant_id": "default", "repository": repo },
        "payload": { "username": "admin", "password": PASSWORD }
    });
    ws_send(&mut socket, &auth).await?;
    let reply = ws_reply(&mut socket).await?;
    if reply["status"] != "success" {
        return Err(format!("ws authenticate: {reply}"));
    }

    let request = json!({
        "request_id": "sql-1",
        "type": "sql_query",
        "context": { "tenant_id": "default", "repository": repo, "branch": branch },
        "payload": { "query": query }
    });
    ws_send(&mut socket, &request).await?;
    let reply = ws_reply(&mut socket).await?;
    let _ = socket.close(None).await;

    if reply["status"] == "error" {
        return Err(format!("ws sql_query: {}", reply["error"]));
    }
    Ok(reply["result"].clone())
}

async fn ws_send<S>(socket: &mut S, envelope: &Value) -> Result<(), String>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let bytes = rmp_serde::to_vec_named(envelope).map_err(|e| format!("ws encode: {e}"))?;
    socket
        .send(Message::Binary(bytes.into()))
        .await
        .map_err(|e| format!("ws send: {e}"))
}

/// The next frame that looks like a response envelope, skipping the connection
/// greeting and any event push that arrives in between.
async fn ws_reply<S>(socket: &mut S) -> Result<Value, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = Duration::from_secs(30);
    loop {
        let message = tokio::time::timeout(deadline, socket.next())
            .await
            .map_err(|_| "ws timed out".to_string())?
            .ok_or_else(|| "ws closed".to_string())?
            .map_err(|e| format!("ws error: {e}"))?;
        match message {
            Message::Binary(data) => {
                let value: Value = rmp_serde::from_slice(&data)
                    .map_err(|e| format!("ws decode: {e} ({} bytes)", data.len()))?;
                if value.get("status").is_some() {
                    return Ok(value);
                }
            }
            Message::Text(text) => return Err(format!("unexpected text frame: {text}")),
            Message::Close(frame) => return Err(format!("ws closed: {frame:?}")),
            _ => continue,
        }
    }
}

// -------------------------------------------------------------------- PGWire

/// Connect a real postgres client to the pgwire listener.
pub async fn pg_connect(
    port: u16,
    repo: &str,
    api_key: &str,
) -> Result<tokio_postgres::Client, String> {
    let conn = format!(
        "host=127.0.0.1 port={port} user=default password={api_key} dbname={repo} \
         connect_timeout=10"
    );
    let (client, connection) = tokio_postgres::connect(&conn, tokio_postgres::NoTls)
        .await
        .map_err(|e| format!("pgwire connect: {e}"))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("pgwire connection closed: {e}");
        }
    });
    Ok(client)
}

/// Render a `tokio_postgres::Error` with the server's message, which the plain
/// `Display` impl hides behind the useless string "db error".
pub fn pg_error(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        return format!("{}: {}", db.severity(), db.message());
    }
    let mut out = e.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(cause) = source {
        out.push_str(&format!(" <- {cause}"));
        source = cause.source();
    }
    out
}

/// Give a pgwire session the privileges of an HTTP admin.
///
/// A pgwire connection authenticates with an API key, but the connection's
/// `AuthContext` is left EMPTY — `execute_query` only ever receives the identity
/// set by `SET app.user`, so the admin identity that the API key proves is dropped
/// on the floor and every statement runs effectively anonymous. Reads survive that;
/// writes do not (`RLS: no permissions in auth context - denying create`).
///
/// The supported way to give a pgwire session an identity is therefore
/// `SET app.user = '<jwt>'`, which resolves a `raisin:User` node in
/// `raisin:access_control` by its `user_id` property. So this creates such a node
/// carrying the *admin's own* `user_id` and the `system_admin` role, and the caller
/// then presents the genuine server-issued admin JWT — no forged token, and the
/// pgwire session ends up with exactly the privileges the HTTP session already had.
pub async fn grant_pgwire_identity(base_url: &str, token: &str, repo: &str, user_id: &str) {
    let sql = format!(
        "INSERT INTO 'raisin:access_control' (id, path, node_type, properties) VALUES \
         ('pgwire-test-user', '/pgwire-test-user', 'raisin:User', \
          '{{\"user_id\":\"{user_id}\",\"display_name\":\"pgwire test\",\
             \"email\":\"pgwire-test@example.invalid\",\"status\":\"active\",\
             \"roles\":[\"system_admin\"]}}'::JSONB)"
    );
    sql_http(base_url, token, repo, &sql)
        .await
        .unwrap_or_else(|e| panic!("creating the pgwire identity user failed: {e}"));
}

/// Wait until the pgwire listener accepts a connection (it is spawned after the
/// HTTP health endpoint goes live).
pub async fn pg_wait_ready(port: u16, repo: &str, api_key: &str) -> tokio_postgres::Client {
    let start = std::time::Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(30) {
        match pg_connect(port, repo, api_key).await {
            Ok(client) => return client,
            Err(e) => {
                last = e;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
    panic!("pgwire never became ready on port {port}: {last}");
}
