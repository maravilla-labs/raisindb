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

//! Executing a proxy function: forwarding a call to a remote MCP server.
//!
//! A proxy `raisin:Function` has no code. Its `mcp_proxy` block names a
//! `raisin:McpConnection` and a remote tool, and invoking it means one
//! `tools/call` against that server.
//!
//! This sits behind the SINGLE branch in
//! [`execute_function`](super::executor::execute_function), which is the one
//! funnel all three execution paths reach — the JS chat path (via the
//! `AIToolCall` job), `raisin.functions.execute()`, and the Rust flow runtime.
//! One implementation, so the paths cannot drift.

use std::sync::Arc;
use std::time::Duration;

use raisin_mcp_protocol::client::McpClientConfig;
use raisin_mcp_protocol::client::{
    concat_text, ensure_object, shared_session_cache, McpClientSession, McpConnectionDescriptor,
    RemoteToolError, StreamableHttpTransport,
};
use raisin_mcp_protocol::protocol::CallToolResult;
use raisin_mcp_protocol::ContentBlock;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::{json, Value};

use super::types::{ExecutionDependencies, FunctionExecutionResult};
use raisin_error::Result;

/// Workspace holding connection configuration.
const SYSTEM_WORKSPACE: &str = "raisin:system";

/// Operator settings for calling other servers' MCP tools, set once at startup.
///
/// Process-global rather than a field on [`ExecutionDependencies`] on purpose.
/// That struct is rebuilt at five call sites plus once inline for nested calls,
/// and the value is the same for all of them — it comes from the `[mcp_client]`
/// TOML section, not from the request. Threading it would add a sixth place to
/// forget, and forgetting would silently downgrade one path's egress policy.
/// Same reasoning as [`shared_http_client`](super::shared_http_client).
static MCP_CLIENT_CONFIG: std::sync::OnceLock<McpClientConfig> = std::sync::OnceLock::new();

/// Install the operator's `[mcp_client]` settings. Called once, from the server
/// binary; later calls are ignored.
pub fn configure_mcp_client(config: McpClientConfig) {
    let _ = MCP_CLIENT_CONFIG.set(config);
}

/// Register the session cache with the derived-cache registry, once.
///
/// Checkpoint ingestion copies whole column families straight into the live
/// database and deliberately emits no events, so every event-driven cache is
/// silently wrong afterwards. A session cache that survived one would keep
/// talking to whatever the pre-checkpoint connection pointed at.
fn register_session_cache() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        raisin_core::services::derived_cache_registry::register_invalidator(|| {
            shared_session_cache().clear();
        });
    });
}

/// The configured settings, or the safe default.
///
/// The default refuses loopback and private addresses, so a deployment that
/// never calls [`configure_mcp_client`] cannot be talked into dialling its own
/// network — it simply cannot reach a local sidecar.
fn mcp_client_config() -> &'static McpClientConfig {
    MCP_CLIENT_CONFIG.get_or_init(McpClientConfig::default)
}

/// Cap on a single base64 payload echoed back into a tool result.
///
/// An unbounded image would land in a `raisin:AIToolSingleCallResult` node
/// property and then in the NEXT model prompt — blowing the context window and
/// costing real money, for bytes the model cannot see anyway.
const MAX_INLINE_BINARY_BYTES: usize = 256 * 1024;

/// Whether a function node is a remote MCP proxy.
///
/// Presence of `mcp_proxy` is the discriminator, deliberately. `execution_mode`
/// cannot be: an unrecognised value silently parses as `Async`, so a "remote"
/// mode would be indistinguishable from a normal function. `language` cannot
/// be either: an unrecognised value is a hard error, and the check has to run
/// BEFORE the language is read.
pub fn proxy_block(node: &Node) -> Option<Value> {
    let value = node
        .properties
        .get("mcp_proxy")
        .and_then(|pv| serde_json::to_value(pv).ok())?;
    value.is_object().then_some(value)
}

/// Invoke a proxy function's remote tool.
// Mirrors `execute_function`'s parameter list on purpose: this is the branch
// taken out of it, and reshuffling the arguments into a struct here would make
// the two harder to read together, not easier.
#[allow(clippy::too_many_arguments)]
pub async fn invoke_remote_tool<S, B>(
    deps: &ExecutionDependencies<S, B>,
    proxy: &Value,
    function_path: &str,
    execution_id: &str,
    input: Value,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    start_time: std::time::Instant,
) -> Result<FunctionExecutionResult>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let finish = |success: bool, result: Option<Value>, error: Option<String>| {
        Ok(FunctionExecutionResult {
            execution_id: execution_id.to_string(),
            success,
            result,
            error,
            duration_ms: start_time.elapsed().as_millis() as u64,
            logs: Vec::new(),
        })
    };

    let Some(remote_tool) = proxy.get("remote_tool").and_then(Value::as_str) else {
        return finish(
            false,
            None,
            Some(format!(
                "proxy function `{function_path}` has no `mcp_proxy.remote_tool`"
            )),
        );
    };
    let Some(connection_path) = proxy.get("connection").and_then(Value::as_str) else {
        return finish(
            false,
            None,
            Some(format!(
                "proxy function `{function_path}` has no `mcp_proxy.connection`"
            )),
        );
    };

    // Load the connection. Read with the SAME storage the caller is scoped to,
    // so a proxy can only ever reach its own tenant's connections.
    let node = match load_connection(deps, tenant_id, repo_id, branch, connection_path).await {
        Ok(Some(node)) => node,
        Ok(None) => {
            return finish(
                false,
                None,
                Some(format!(
                    "proxy function `{function_path}` references a missing connection \
                     `{connection_path}`; run tool discovery or re-create the connection"
                )),
            )
        }
        Err(e) => return finish(false, None, Some(e.to_string())),
    };

    let descriptor = match McpConnectionDescriptor::from_node(&node) {
        Ok(d) => d,
        Err(e) => return finish(false, None, Some(e.to_string())),
    };
    if !descriptor.is_callable() {
        return finish(
            false,
            None,
            Some(format!("MCP connection `{}` is disabled", descriptor.slug)),
        );
    }

    let config = mcp_client_config();

    // Re-validated at CALL time, not only at save time: a hostname that
    // resolved publicly when the connection was saved can be re-pointed at a
    // private address afterwards.
    if let Err(e) = config.egress_policy().validate_url(&descriptor.url) {
        return finish(false, None, Some(e.to_string()));
    }

    let arguments = match ensure_object(input) {
        Ok(args) => args,
        Err(e) => return finish(false, None, Some(e.to_string())),
    };

    let credential = decrypt_credential(&node, &descriptor);
    let headers = descriptor.auth_headers(credential.as_deref());

    let timeout = Duration::from_millis(
        descriptor
            .refresh_policy
            .call_timeout_ms
            .min(config.default_timeout_ms.max(1)),
    );
    // Reuse the negotiated session: without it every call pays a handshake
    // round trip before the one that does the work.
    register_session_cache();
    let http = deps.http_client.clone();
    let url = descriptor.url.clone();
    let max_bytes = config.max_response_bytes;
    let session = shared_session_cache().get_or_insert(
        tenant_id,
        repo_id,
        branch,
        &descriptor.slug,
        descriptor.url.as_str(),
        move || {
            McpClientSession::new(
                StreamableHttpTransport::new(http, url, timeout).with_max_response_bytes(max_bytes),
            )
        },
    );

    match session.call_tool(remote_tool, arguments, &headers).await {
        Ok(result) => {
            let (success, value, error) = map_result(result);
            finish(success, value, error)
        }
        // A TRANSPORT failure is an execution failure, distinct from a tool that
        // ran and reported a domain error (which `map_result` turns into
        // `success: false` with a message the model can act on).
        Err(err) => finish(false, None, Some(describe(&err, remote_tool))),
    }
}

/// Map a `CallToolResult` onto the function-result shape.
///
/// Returns `(success, result, error)`. `isError: true` becomes
/// `success: false` — NOT a transport error — so both the JS path (which writes
/// an `error` property the model sees on its next turn) and the flow path
/// (which turns it into `{"error": …}`) produce identical shapes.
pub fn map_result(result: CallToolResult) -> (bool, Option<Value>, Option<String>) {
    if result.is_error {
        let message = concat_text(&result);
        let message = if message.is_empty() {
            "remote tool reported an error".to_string()
        } else {
            message
        };
        return (false, None, Some(message));
    }

    // A tool declaring an outputSchema gives us the machine-readable form
    // directly; prefer it over re-parsing text.
    if let Some(structured) = result.structured_content {
        return (true, Some(structured), None);
    }

    match result.content.len() {
        0 => (true, Some(json!({})), None),
        1 => {
            let block = &result.content[0];
            match block.as_text() {
                // Most servers return JSON serialized into a text block.
                Some(text) => match serde_json::from_str::<Value>(text) {
                    Ok(parsed) => (true, Some(parsed), None),
                    Err(_) => (true, Some(json!({ "text": text })), None),
                },
                None => (true, Some(json!({ "content": [normalize(block)] })), None),
            }
        }
        _ => {
            let blocks: Vec<Value> = result.content.iter().map(normalize).collect();
            (true, Some(json!({ "content": blocks })), None)
        }
    }
}

/// Render one content block, truncating inline binary payloads.
fn normalize(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
        ContentBlock::Json { json } => json!({ "type": "json", "json": json }),
        ContentBlock::Image { data, mime_type } => binary_block("image", data, mime_type),
        ContentBlock::Audio { data, mime_type } => binary_block("audio", data, mime_type),
        ContentBlock::ResourceLink {
            uri,
            name,
            mime_type,
        } => json!({
            "type": "resource_link",
            "uri": uri,
            "name": name,
            "mimeType": mime_type,
        }),
        ContentBlock::Resource { resource } => {
            let mut out = json!({
                "type": "resource",
                "uri": resource.uri,
                "mimeType": resource.mime_type,
            });
            if let Some(text) = &resource.text {
                out["text"] = json!(text);
            }
            if let Some(blob) = &resource.blob {
                out["blob"] = truncated(blob);
            }
            out
        }
        ContentBlock::Other { block_type, raw } => {
            json!({ "type": block_type, "raw": raw })
        }
    }
}

/// An image/audio block with its payload bounded.
fn binary_block(kind: &str, data: &str, mime_type: &str) -> Value {
    json!({ "type": kind, "mimeType": mime_type, "data": truncated(data) })
}

/// Replace an oversized base64 payload with a marker.
fn truncated(data: &str) -> Value {
    if data.len() <= MAX_INLINE_BINARY_BYTES {
        return json!(data);
    }
    json!({ "truncated": true, "bytes": data.len() })
}

/// A human-readable description of a transport failure.
fn describe(err: &RemoteToolError, tool: &str) -> String {
    format!("remote MCP tool `{tool}` failed ({}): {err}", err.code())
}

/// Load a connection node from the `raisin:system` workspace.
async fn load_connection<S, B>(
    deps: &ExecutionDependencies<S, B>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    connection_path: &str,
) -> Result<Option<Node>>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let mut auth = raisin_models::auth::AuthContext::system();
    auth.user_id = Some("mcp-proxy".to_string());
    let svc: raisin_core::NodeService<S> = raisin_core::NodeService::new_with_context(
        deps.storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
        SYSTEM_WORKSPACE.to_string(),
    )
    .with_auth(auth);
    svc.get_by_path(connection_path).await
}

/// Decrypt the credential the connection presents, if any.
fn decrypt_credential(node: &Node, descriptor: &McpConnectionDescriptor) -> Option<String> {
    use raisin_mcp_protocol::client::AuthKind;

    let (property, field) = match descriptor.auth_kind {
        AuthKind::None => return None,
        AuthKind::Static => ("credential_encrypted", "value"),
        AuthKind::Oauth => ("tokens_encrypted", "access_token"),
    };
    let ciphertext = match node.properties.get(property) {
        Some(PropertyValue::String(s)) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let master_key = raisin_crypto::master_key_with_embedding_fallback()
        .ok()
        .flatten()?;

    let plaintext = raisin_crypto::SecretBox::new(&master_key)
        .decrypt_json(&ciphertext)
        .ok()
        .and_then(|v| {
            v.get(field)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        });
    if plaintext.is_none() {
        tracing::error!(
            node_path = %node.path,
            property,
            "mcp connection credential could not be decrypted; the call will proceed \
             UNAUTHENTICATED and the remote will reject it (RAISIN_MASTER_KEY mismatch?)"
        );
    }
    plaintext
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_mcp_protocol::protocol::RESULT_TYPE_COMPLETE;

    fn result(content: Vec<ContentBlock>, is_error: bool) -> CallToolResult {
        CallToolResult {
            result_type: RESULT_TYPE_COMPLETE.to_string(),
            content,
            is_error,
            structured_content: None,
        }
    }

    #[test]
    fn structured_content_wins() {
        let mut r = result(vec![ContentBlock::text("ignored")], false);
        r.structured_content = Some(json!({ "count": 2 }));
        assert_eq!(map_result(r), (true, Some(json!({ "count": 2 })), None));
    }

    #[test]
    fn a_single_json_text_block_is_parsed() {
        let r = result(vec![ContentBlock::text("{\"ok\":true}")], false);
        assert_eq!(map_result(r), (true, Some(json!({ "ok": true })), None));
    }

    #[test]
    fn a_single_non_json_text_block_becomes_a_text_field() {
        let r = result(vec![ContentBlock::text("hello")], false);
        assert_eq!(
            map_result(r),
            (true, Some(json!({ "text": "hello" })), None)
        );
    }

    #[test]
    fn multiple_blocks_become_a_content_array() {
        let r = result(
            vec![ContentBlock::text("a"), ContentBlock::text("b")],
            false,
        );
        let (ok, value, err) = map_result(r);
        assert!(ok && err.is_none());
        assert_eq!(value.unwrap()["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn an_error_result_is_a_failure_not_a_transport_error() {
        let r = result(vec![ContentBlock::text("no such issue")], true);
        let (ok, value, err) = map_result(r);
        // `success: false` with a message, so the model can see and correct it.
        assert!(!ok);
        assert!(value.is_none());
        assert_eq!(err.as_deref(), Some("no such issue"));
    }

    #[test]
    fn an_empty_error_result_still_carries_a_message() {
        let (_, _, err) = map_result(result(Vec::new(), true));
        assert!(err.is_some_and(|e| !e.is_empty()));
    }

    #[test]
    fn an_empty_success_result_is_an_empty_object() {
        assert_eq!(map_result(result(Vec::new(), false)).1, Some(json!({})));
    }

    #[test]
    fn oversized_binary_payloads_are_truncated() {
        let big = "A".repeat(MAX_INLINE_BINARY_BYTES + 1);
        let r = result(
            vec![
                ContentBlock::text("see image"),
                ContentBlock::Image {
                    data: big.clone(),
                    mime_type: "image/png".into(),
                },
            ],
            false,
        );
        let (_, value, _) = map_result(r);
        let blocks = value.unwrap();
        let image = &blocks["content"][1];
        // The bytes must NOT survive into a node property and the next prompt.
        assert_eq!(image["data"]["truncated"], json!(true));
        assert!(!image.to_string().contains(&big));
    }

    #[test]
    fn small_binary_payloads_are_preserved() {
        let small = "A".repeat(16);
        let r = result(
            vec![
                ContentBlock::Image {
                    data: small.clone(),
                    mime_type: "image/png".into(),
                },
                ContentBlock::text("x"),
            ],
            false,
        );
        let (_, value, _) = map_result(r);
        assert_eq!(value.unwrap()["content"][0]["data"], json!(small));
    }

    #[test]
    fn an_unknown_block_type_is_carried_through() {
        let raw = json!({ "type": "video", "url": "https://x/y.mp4" });
        let block: ContentBlock = serde_json::from_value(raw.clone()).unwrap();
        let r = result(vec![block, ContentBlock::text("x")], false);
        let (_, value, _) = map_result(r);
        assert_eq!(value.unwrap()["content"][0]["raw"], raw);
    }

    #[test]
    fn proxy_block_requires_an_object() {
        let mut node = Node::default();
        assert!(proxy_block(&node).is_none());

        node.properties.insert(
            "mcp_proxy".to_string(),
            serde_json::from_value(json!({ "remote_tool": "x" })).unwrap(),
        );
        assert!(proxy_block(&node).is_some());
    }
}
