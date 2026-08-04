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

//! Refreshing an MCP connection's OAuth access token.
//!
//! Runs inside the existing `IntegrationTokenRefresh` sweep rather than as a
//! second job: it scans the same `raisin:system` workspace on the same
//! interval, and one job with two node types cannot drift out of step the way
//! two jobs on two schedules would.
//!
//! The refresh token is decrypted here and never crosses into a function
//! sandbox, an API response or a log line.

use std::sync::Arc;
use std::time::Duration;

use raisin_core::services::node_service::NodeService;
use raisin_crypto::SecretBox;
use raisin_error::Result;
use raisin_locks::LockManagerHandle;
use raisin_mcp_protocol::client::{self as mcp, AuthServerMetadata, RegisteredClient};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::{json, Value};

use crate::RocksDBStorage;

/// Refresh a token this many seconds before it actually expires.
const REFRESH_THRESHOLD_SECS: i64 = 30 * 60;

/// How long one connection's refresh lease is held.
const REFRESH_LEASE_TTL: Duration = Duration::from_secs(120);

/// NodeType of an MCP connection.
const CONNECTION_NODE_TYPE: &str = "raisin:McpConnection";

/// Refresh every expiring MCP connection in one repo.
///
/// Returns how many connections had their tokens rotated. Mirrors the
/// integration path exactly, including the per-node lease: two nodes presenting
/// the same rotating refresh token concurrently is how an account ends up
/// disconnected, and two nodes writing the same node lose one another's update.
pub async fn refresh_repo(
    storage: &Arc<RocksDBStorage>,
    svc: &NodeService<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant: &str,
    repo: &str,
    branch: &str,
    secret_box: &SecretBox,
    now_secs: i64,
) -> Result<usize> {
    let _ = storage;
    let mut refreshed = 0usize;

    for node in svc.list_by_type(CONNECTION_NODE_TYPE).await? {
        if !is_expiring(&node, now_secs) {
            continue;
        }
        let lock_key = raisin_locks::scoped_key(
            tenant,
            repo,
            branch,
            &format!("mcp-token-refresh:{}", node.id),
        );
        let lease = match lock_manager {
            Some(lm) => {
                let owner = format!("{instance_id}:{}", node.id);
                match lm.try_acquire(&lock_key, &owner, REFRESH_LEASE_TTL).await? {
                    Some(guard) => Some(guard.token),
                    None => continue,
                }
            }
            None => None,
        };

        let outcome = refresh_node(svc, node, secret_box, now_secs).await;

        if let (Some(lm), Some(token)) = (lock_manager, lease) {
            let _ = lm.release(&lock_key, token).await;
        }
        match outcome {
            Ok(true) => refreshed += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "mcp token refresh: connection failed"),
        }
    }
    Ok(refreshed)
}

/// Whether a connection's access token is close enough to expiry to rotate.
///
/// A connection with no recorded expiry is left alone: we cannot know when it
/// lapses, and refreshing on every sweep would burn the provider's quota. A
/// 401 at call time drives the reconnect instead.
pub fn is_expiring(node: &Node, now_secs: i64) -> bool {
    let has_tokens = matches!(
        node.properties.get("tokens_encrypted"),
        Some(PropertyValue::String(s)) if !s.is_empty()
    );
    if !has_tokens {
        return false;
    }
    match number_prop(node, "expires_at") {
        Some(expiry) => expiry <= now_secs + REFRESH_THRESHOLD_SECS,
        None => false,
    }
}

/// Rotate one connection's tokens. Returns whether anything was written.
async fn refresh_node(
    svc: &NodeService<RocksDBStorage>,
    mut node: Node,
    secret_box: &SecretBox,
    now_secs: i64,
) -> Result<bool> {
    let cfg = json_prop(&node, "oauth_client");
    let metadata = AuthServerMetadata {
        issuer: json_str(&cfg, "issuer").unwrap_or_default(),
        authorization_endpoint: json_str(&cfg, "authorization_endpoint").unwrap_or_default(),
        token_endpoint: json_str(&cfg, "token_endpoint").unwrap_or_default(),
        registration_endpoint: None,
        code_challenge_methods_supported: Vec::new(),
        scopes_supported: Vec::new(),
    };
    if metadata.token_endpoint.is_empty() {
        return Ok(false);
    }

    let Some(refresh_token) = decrypt_field(&node, "tokens_encrypted", "refresh_token", secret_box)
    else {
        // An access token with no refresh token cannot be rotated; it simply
        // lapses and the operator reconnects.
        return Ok(false);
    };

    let client = RegisteredClient {
        client_id: json_str(&cfg, "client_id").unwrap_or_default(),
        client_secret: decrypt_field(&node, "oauth_client_secret_encrypted", "value", secret_box),
    };

    // The token endpoint comes from metadata the REMOTE side supplied at
    // discovery time, so it is guarded here too — this POST carries the refresh
    // token and any client secret.
    let tokens = match mcp::refresh_tokens(
        &raisin_functions_http_client(),
        &mcp::installed_egress_policy(),
        &metadata,
        &client,
        &refresh_token,
        json_str(&cfg, "resource").as_deref(),
    )
    .await
    {
        Ok(tokens) => tokens,
        Err(e) => {
            // Record it so the console can say "reconnect" rather than leaving
            // an operator to discover it through failing tool calls.
            set_prop(
                &mut node,
                "health",
                json!({
                    "status": "auth_expired",
                    "checked_at": chrono::Utc::now().to_rfc3339(),
                    "error_code": e.code(),
                    "error": e.to_string(),
                }),
            )?;
            svc.update_node(node).await?;
            return Ok(false);
        }
    };

    // Providers that rotate refresh tokens issue a new one; those that do not
    // omit it, and the existing one stays valid.
    let refresh = tokens
        .refresh_token
        .clone()
        .unwrap_or_else(|| refresh_token.clone());
    let blob = secret_box
        .encrypt_json(&json!({
            "access_token": tokens.access_token,
            "refresh_token": refresh,
        }))
        .map_err(|e| raisin_error::Error::Internal(format!("failed to encrypt MCP tokens: {e}")))?;
    set_prop(&mut node, "tokens_encrypted", json!(blob))?;

    match tokens.expires_in {
        Some(ttl) => set_prop(&mut node, "expires_at", json!(now_secs.max(0) as u64 + ttl))?,
        None => {
            node.properties.remove("expires_at");
        }
    }
    svc.update_node(node).await?;
    Ok(true)
}

/// The process-wide HTTP client.
fn raisin_functions_http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

fn decrypt_field(
    node: &Node,
    property: &str,
    field: &str,
    secret_box: &SecretBox,
) -> Option<String> {
    let blob = match node.properties.get(property) {
        Some(PropertyValue::String(s)) if !s.is_empty() => s,
        _ => return None,
    };
    secret_box
        .decrypt_json(blob)
        .ok()?
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn json_prop(node: &Node, key: &str) -> Value {
    node.properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
}

fn json_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn number_prop(node: &Node, key: &str) -> Option<i64> {
    let value = node
        .properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())?;
    value.as_i64().or_else(|| value.as_f64().map(|n| n as i64))
}

fn set_prop(node: &mut Node, key: &str, value: Value) -> Result<()> {
    let pv = serde_json::from_value::<PropertyValue>(value)
        .map_err(|e| raisin_error::Error::Validation(format!("failed to encode `{key}`: {e}")))?;
    node.properties.insert(key.to_string(), pv);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_with(tokens: bool, expires_at: Option<i64>) -> Node {
        let mut node = Node::default();
        if tokens {
            node.properties.insert(
                "tokens_encrypted".into(),
                PropertyValue::String("cipher".into()),
            );
        }
        if let Some(e) = expires_at {
            node.properties.insert(
                "expires_at".into(),
                serde_json::from_value(json!(e)).unwrap(),
            );
        }
        node
    }

    #[test]
    fn refreshes_inside_the_threshold_window() {
        let now = 1_000_000;
        assert!(is_expiring(&node_with(true, Some(now + 600)), now));
        assert!(is_expiring(&node_with(true, Some(now - 5)), now));
        assert!(!is_expiring(&node_with(true, Some(now + 7200)), now));
    }

    #[test]
    fn a_connection_without_tokens_is_never_refreshed() {
        let now = 1_000_000;
        assert!(!is_expiring(&node_with(false, Some(now)), now));
    }

    #[test]
    fn a_missing_expiry_does_not_trigger_a_refresh_every_sweep() {
        // We cannot know when it lapses; a 401 at call time drives the reconnect.
        assert!(!is_expiring(&node_with(true, None), 1_000_000));
    }
}
